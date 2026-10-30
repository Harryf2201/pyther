//! Lockless transposition table (safe to share between search threads).
//! Each entry is two atomics: `key ^ data` and `data`, so a torn write is detected on probe.

use crate::params::{p, P};
use crate::types::Move;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering::Relaxed};

pub const BOUND_NONE: u8 = 0;
pub const BOUND_UPPER: u8 = 1;
pub const BOUND_LOWER: u8 = 2;
pub const BOUND_EXACT: u8 = 3;

#[derive(Clone, Copy)]
pub struct TTData {
    pub mv: Move,
    pub score: i32,
    pub eval: i32,
    pub depth: i32,
    pub bound: u8,
    /// The position was on a principal variation when stored (or earlier).
    pub pv: bool,
}

#[derive(Default)]
#[repr(C, align(16))]
struct Entry {
    key: AtomicU64,
    data: AtomicU64,
}

pub struct TT {
    entries: Vec<Entry>,
    age: AtomicU8,
    /// Sprint 13 (SprTtBucket): the first entry on a 64-byte boundary and the number of
    /// 4-entry clusters (one cache line each) from there.
    base: usize,
    clusters: usize,
}

// data layout: mv 0-15 | score 16-31 | eval 32-47 | depth 48-55 | bound 56-57 | age 58-62 | pv 63
fn pack(mv: Move, score: i32, eval: i32, depth: i32, bound: u8, age: u8, pv: bool) -> u64 {
    mv.0 as u64
        | ((score as i16 as u16 as u64) << 16)
        | ((eval as i16 as u16 as u64) << 32)
        | ((depth.clamp(0, 255) as u64) << 48)
        | ((bound as u64) << 56)
        | (((age & 31) as u64) << 58)
        | ((pv as u64) << 63)
}

impl TT {
    pub fn new(mb: usize) -> TT {
        let count = (mb.max(1) * 1024 * 1024 / std::mem::size_of::<Entry>()).max(1);
        let mut entries = Vec::with_capacity(count);
        entries.resize_with(count, Entry::default);
        let addr = entries.as_ptr() as usize;
        let base = ((64 - addr % 64) % 64) / std::mem::size_of::<Entry>();
        let clusters = count.saturating_sub(base) / 4;
        TT { entries, age: AtomicU8::new(0), base, clusters }
    }

    #[inline(always)]
    fn index(&self, key: u64) -> usize {
        ((key as u128 * self.entries.len() as u128) >> 64) as usize
    }

    /// Sprint 13: 4-way clusters instead of one entry per index.
    #[inline(always)]
    fn bucketed(&self) -> bool {
        p(P::SprTtBucket) > 0 && self.clusters > 0
    }

    /// First entry of the key's cluster (bucketed mode).
    #[inline(always)]
    fn cluster(&self, key: u64) -> usize {
        self.base + 4 * ((key as u128 * self.clusters as u128) >> 64) as usize
    }

    #[inline(always)]
    fn read(e: &Entry, key: u64) -> Option<TTData> {
        let data = e.data.load(Relaxed);
        if e.key.load(Relaxed) ^ data != key {
            return None;
        }
        let bound = ((data >> 56) & 3) as u8;
        if bound == BOUND_NONE {
            return None;
        }
        Some(TTData {
            mv: Move(data as u16),
            score: (data >> 16) as u16 as i16 as i32,
            eval: (data >> 32) as u16 as i16 as i32,
            depth: ((data >> 48) & 0xFF) as i32,
            bound,
            pv: data >> 63 != 0,
        })
    }

    #[inline(always)]
    fn write(e: &Entry, key: u64, data: u64) {
        e.data.store(data, Relaxed);
        e.key.store(key ^ data, Relaxed);
    }

    pub fn clear(&self) {
        for e in &self.entries {
            e.key.store(0, Relaxed);
            e.data.store(0, Relaxed);
        }
        self.age.store(0, Relaxed);
    }

    pub fn new_search(&self) {
        self.age.store(self.age.load(Relaxed).wrapping_add(1) & 31, Relaxed);
    }

    pub fn prefetch(&self, key: u64) {
        let i = if self.bucketed() { self.cluster(key) } else { self.index(key) };
        #[cfg(target_arch = "x86_64")]
        unsafe {
            use core::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
            _mm_prefetch(&self.entries[i] as *const Entry as *const i8, _MM_HINT_T0);
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            let ptr = &self.entries[i] as *const Entry;
            core::arch::asm!("prfm pldl1keep, [{0}]", in(reg) ptr, options(readonly, nostack, preserves_flags));
        }
    }

    pub fn probe(&self, key: u64) -> Option<TTData> {
        if self.bucketed() {
            let c = self.cluster(key);
            return self.entries[c..c + 4].iter().find_map(|e| Self::read(e, key));
        }
        Self::read(&self.entries[self.index(key)], key)
    }

    pub fn store(&self, key: u64, mut mv: Move, score: i32, eval: i32, depth: i32, bound: u8, pv: bool) {
        let age = self.age.load(Relaxed);
        if self.bucketed() {
            // The position's own entry keeps the single-entry update rule; a new position replaces an
            // empty entry, else the one with the lowest depth - 8 * (searches since it was written).
            let c = self.cluster(key);
            let mut victim = c;
            let mut victim_value = i32::MAX;
            for i in c..c + 4 {
                let e = &self.entries[i];
                let old_data = e.data.load(Relaxed);
                let old_bound = (old_data >> 56) & 3;
                let old_depth = ((old_data >> 48) & 0xFF) as i32;
                let old_age = ((old_data >> 58) & 31) as u8;
                if old_bound != 0 && e.key.load(Relaxed) ^ old_data == key {
                    if mv.is_null() {
                        mv = Move(old_data as u16);
                    }
                    if bound == BOUND_EXACT || depth + 4 > old_depth || old_age != age {
                        Self::write(e, key, pack(mv, score, eval, depth, bound, age, pv));
                    }
                    return;
                }
                let value = if old_bound == 0 { i32::MIN } else { old_depth - 8 * (age as i32 - old_age as i32).rem_euclid(32) };
                if value < victim_value {
                    victim_value = value;
                    victim = i;
                }
            }
            Self::write(&self.entries[victim], key, pack(mv, score, eval, depth, bound, age, pv));
            return;
        }
        let e = &self.entries[self.index(key)];
        let old_data = e.data.load(Relaxed);
        let same = e.key.load(Relaxed) ^ old_data == key;
        if same && mv.is_null() {
            mv = Move(old_data as u16);
        }
        let old_depth = ((old_data >> 48) & 0xFF) as i32;
        let old_age = ((old_data >> 58) & 31) as u8;
        if !same || bound == BOUND_EXACT || depth + 4 > old_depth || old_age != age {
            Self::write(e, key, pack(mv, score, eval, depth, bound, age, pv));
        }
    }

    /// Permill of sampled entries written during the current search.
    pub fn hashfull(&self) -> usize {
        let age = self.age.load(Relaxed);
        self.entries
            .iter()
            .take(1000)
            .filter(|e| {
                let d = e.data.load(Relaxed);
                (d >> 56) & 3 != 0 && ((d >> 58) & 31) as u8 == age
            })
            .count()
    }
}
