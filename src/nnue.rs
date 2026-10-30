//! NNUE evaluation, multi-layer (trainer -Inputs ml): (768 -> HIDDEN, clipped ReLU, pairwise products of the
//! two halves) x2 -> L1 -> L2 -> 1, every layer after the first chosen by material (OUTPUT_BUCKETS).
//! The first layer is i16 (incrementally updated accumulators); the small layers run in f32.
//!
//! Each side has an accumulator (the first layer's output). Moves only change a few input
//! features, so the search updates accumulators incrementally instead of recomputing them.
//! The default network is embedded from nets/default.bin; an empty file means "no network",
//! in which case the engine falls back to the hand-crafted evaluation.

use crate::board::Board;
use crate::types::*;

/// Must match the trainer's hidden size and bucket count (trainer args 6 and 7).
pub const HIDDEN: usize = 1024;
pub const OUTPUT_BUCKETS: usize = 8;
const L1: usize = 16;
const L2: usize = 32;
const SCALE: f32 = 400.0;
const QA: i32 = 255;

#[cfg(feature = "kb4nm")]
pub const INPUT_BUCKETS: usize = 4;
#[cfg(not(feature = "kb4nm"))]
pub const INPUT_BUCKETS: usize = 1;
pub const NET_FILE_BYTES: usize = (NET_BYTES + 63) & !63;
#[cfg(feature = "kb4nm")]
const ARCHITECTURE: &str = "inputs=mlkb4nm\nhidden=1024\noutput_buckets=8\nresidual_scale=0.25\nmirrored=false\n";
#[cfg(feature = "kb4nm")]
static NET_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Four own-king zones, preserving absolute files. Black only rank-flips.
#[cfg(feature = "kb4nm")]
#[inline(always)]
fn king_zone(ksq: usize) -> usize {
    match ksq / 8 {
        0 => usize::from((2..=5).contains(&(ksq % 8))),
        1 => 2,
        _ => 3,
    }
}
#[cfg(feature = "kb4nm")]
#[inline(always)]
fn persp_zone(b: &Board, persp: usize) -> usize {
    let ksq = (b.pieces[KING] & b.colors[persp]).trailing_zeros() as usize;
    king_zone(ksq ^ if persp == BLACK { 56 } else { 0 })
}

/// Size of bullet's quantised output (it may be padded to a multiple of 64).
pub const NET_BYTES: usize = (768 * INPUT_BUCKETS * HIDDEN + HIDDEN) * 2
    + (HIDDEN * OUTPUT_BUCKETS * L1 + OUTPUT_BUCKETS * L1 + L1 * OUTPUT_BUCKETS * L2 + OUTPUT_BUCKETS * L2 + L2 * OUTPUT_BUCKETS + OUTPUT_BUCKETS) * 4;

#[derive(Clone, Copy)]
#[repr(C, align(64))]
pub struct Accumulator {
    pub vals: [i16; HIDDEN],
}

#[repr(C)]
pub struct Network {
    feature_weights: [Accumulator; 768 * INPUT_BUCKETS],
    feature_bias: Accumulator,
    // bullet's native (input-major) layout: for each input, the outputs of every bucket in a row.
    l1w: [[[f32; L1]; OUTPUT_BUCKETS]; HIDDEN],
    l1b: [[f32; L1]; OUTPUT_BUCKETS],
    l2w: [[[f32; L2]; OUTPUT_BUCKETS]; L1],
    l2b: [[f32; L2]; OUTPUT_BUCKETS],
    l3w: [[f32; OUTPUT_BUCKETS]; L2],
    l3b: [f32; OUTPUT_BUCKETS],
}

static DEFAULT_NET: &[u8] = include_bytes!("../nets/default.bin");
static mut NET: Option<&'static Network> = None;

/// Layer 1 quantised to i8 for speed (built from the f32 weights when a network loads), one row of HIDDEN
/// weights per (bucket, output). `mul` turns the integer dot product back into the float pre-activation.
struct L1Quant {
    w: Vec<i8>,
    /// Per (bucket, output) row.
    mul: Vec<f32>,
}
static mut L1Q: Option<&'static L1Quant> = None;

fn set_net(net: &'static Network) {
    let mut w = vec![0i8; OUTPUT_BUCKETS * L1 * HIDDEN];
    let mut mul = vec![0f32; OUTPUT_BUCKETS * L1];
    for b in 0..OUTPUT_BUCKETS {
        for o in 0..L1 {
            // Each row gets its own scale, so small rows keep their precision.
            let max = (0..HIDDEN).fold(0f32, |m, i| m.max(net.l1w[i][b][o].abs())).max(1e-6);
            let scale = 127.0 / max;
            for i in 0..HIDDEN {
                w[(b * L1 + o) * HIDDEN + i] = (net.l1w[i][b][o] * scale).round() as i8;
            }
            // Inputs are round(a * b / 512) with a, b in 0..=255, i.e. x * 65025 / 512.
            mul[b * L1 + o] = 512.0 / (65025.0 * scale);
        }
    }
    unsafe {
        L1Q = Some(Box::leak(Box::new(L1Quant { w, mul })));
        NET = Some(net);
    }
    #[cfg(feature = "kb4nm")]
    NET_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

fn validate_size(len: usize) -> Result<(), String> {
    if len != NET_FILE_BYTES {
        return Err(format!("network architecture/size mismatch: {len} bytes, expected exactly {NET_FILE_BYTES} ({}; 1024/16/32/1, QA255)", if cfg!(feature="kb4nm") { "mlkb4nm" } else { "ml" }));
    }
    Ok(())
}
fn from_bytes(bytes: &[u8]) -> Result<&'static Network, String> {
    validate_size(bytes.len())?;
    let float_offset = (768 * INPUT_BUCKETS * HIDDEN + HIDDEN) * 2;
    if bytes[float_offset..NET_BYTES].chunks_exact(4).any(|v| !f32::from_le_bytes(v.try_into().unwrap()).is_finite()) {
        return Err("network contains nonfinite downstream weights".into());
    }
    unsafe {
        let layout = std::alloc::Layout::new::<Network>();
        let ptr = std::alloc::alloc_zeroed(layout);
        if ptr.is_null() {
            return Err("out of memory".into());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, NET_BYTES.min(layout.size()));
        Ok(&*(ptr as *const Network))
    }
}

/// Loads the embedded network, if one was built in.
pub fn init() {
    match from_bytes(DEFAULT_NET) {
        Ok(net) => {
            set_net(net);
            #[cfg(feature = "kb4nm")]
            println!("info string embedded headerless net accepted as mlkb4nm by build contract; architecture provenance must be checked by the builder");
        }
        Err(_err) => {
            #[cfg(feature = "kb4nm")]
            if !DEFAULT_NET.is_empty() { println!("info string embedded network rejected: {_err}; load a matching KB4nm EvalFile before searching"); }
        }
    }
}

/// Headerless mirrored KB4 has the same length: external KB nets require explicit provenance.
#[cfg(feature = "kb4nm")]
fn validate_architecture(path: &std::path::Path) -> Result<(), String> {
    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let mut candidates = vec![std::path::PathBuf::from(format!("{}.architecture.txt", path.display())), dir.join("architecture.txt")];
    if let Some(parent) = dir.parent() { candidates.push(parent.join("architecture.txt")); }
    for marker in candidates {
        match std::fs::read_to_string(&marker) {
            Ok(text) => {
                if text.replace("\r\n", "\n").trim() != ARCHITECTURE.trim() {
                    return Err("network architecture mismatch: requires mlkb4nm/1024/8/r25/unmirrored; mirrored KB4 is incompatible".into());
                }
                return Ok(());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(format!("cannot read network architecture manifest: {e}")),
        }
    }
    Err("KB4nm network requires <net>.architecture.txt or architecture.txt in its checkpoint/parent directory; cannot distinguish mirrored KB4 by size".into())
}

/// Loads a network from disk (UCI option EvalFile). Not safe to call during a search.
/// A loaded network is never freed (searches hold references to it), so loading the same file again
/// (GUIs and match runners resend options before every game) is skipped instead of leaking a copy.
pub fn load_file(path: &str) -> Result<(), String> {
    static LOADED: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
    let mut loaded = LOADED.lock().unwrap();
    if *loaded == path && active() {
        return Ok(());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    validate_size(bytes.len())?;
    #[cfg(feature = "kb4nm")]
    validate_architecture(std::path::Path::new(path))?;
    let net = from_bytes(&bytes)?;
    set_net(net);
    *loaded = path.to_string();
    Ok(())
}

#[inline(always)]
fn net() -> &'static Network {
    unsafe { NET.unwrap_unchecked() }
}

#[inline(always)]
pub fn active() -> bool {
    unsafe { NET.is_some() }
}

/// Input feature for piece `pc` (color * 6 + type) on `sq`, seen from `persp`'s side.
#[inline(always)]
fn feature(persp: usize, pc: usize, sq: usize) -> usize {
    let (c, pt) = (pc / 6, pc % 6);
    if persp == WHITE {
        if c == WHITE { 64 * pt + sq } else { 384 + 64 * pt + sq }
    } else if c == BLACK {
        64 * pt + (sq ^ 56)
    } else {
        384 + 64 * pt + (sq ^ 56)
    }
}

/// Accumulators for both perspectives, indexed by color.
pub type AccPair = [Accumulator; 2];

pub fn refresh(b: &Board, acc: &mut AccPair) {
    let net = net();
    for (persp, a) in acc.iter_mut().enumerate() {
        #[cfg(feature = "kb4nm")]
        let offset = 768 * persp_zone(b, persp);
        #[cfg(not(feature = "kb4nm"))]
        let offset = 0;
        *a = net.feature_bias;
        for sq in 0..64 {
            let pc = b.mailbox[sq];
            if pc != NO_PIECE {
                let w = &net.feature_weights[offset + feature(persp, pc as usize, sq)].vals;
                for (v, d) in a.vals.iter_mut().zip(w) {
                    *v += *d;
                }
            }
        }
    }
}

#[cfg(feature = "kb4nm")]
#[derive(Clone)]
struct CacheEntry {
    acc: Accumulator,
    bbs: [u64; 12],
}

/// One thread owns eight entries: 2 perspectives x 4 zones, no mirror dimension.
#[cfg(feature = "kb4nm")]
pub struct AccCache {
    generation: u64,
    entries: Box<[CacheEntry]>,
}
#[cfg(feature = "kb4nm")]
impl Default for AccCache {
    fn default() -> Self {
        Self { generation: u64::MAX, entries: vec![CacheEntry { acc: Accumulator { vals: [0; HIDDEN] }, bbs: [0; 12] }; 8].into_boxed_slice() }
    }
}
#[cfg(feature = "kb4nm")]
impl AccCache {
    fn refresh_one(&mut self, b: &Board, persp: usize, out: &mut Accumulator) {
        let net = net();
        let generation = NET_GEN.load(std::sync::atomic::Ordering::Relaxed);
        if generation != self.generation {
            for e in self.entries.iter_mut() { e.acc = net.feature_bias; e.bbs = [0; 12]; }
            self.generation = generation;
        }
        let zone = persp_zone(b, persp);
        let e = &mut self.entries[persp * 4 + zone];
        for pc in 0..12 {
            let cur = b.pieces[pc % 6] & b.colors[pc / 6];
            let (mut add, mut sub) = (cur & !e.bbs[pc], e.bbs[pc] & !cur);
            while add != 0 {
                let w = &net.feature_weights[768 * zone + feature(persp, pc, add.trailing_zeros() as usize)].vals;
                for (v, d) in e.acc.vals.iter_mut().zip(w) { *v += *d; }
                add &= add - 1;
            }
            while sub != 0 {
                let w = &net.feature_weights[768 * zone + feature(persp, pc, sub.trailing_zeros() as usize)].vals;
                for (v, d) in e.acc.vals.iter_mut().zip(w) { *v -= *d; }
                sub &= sub - 1;
            }
            e.bbs[pc] = cur;
        }
        *out = e.acc;
    }
}

#[cfg(feature = "kb4nm")]
fn refresh_one(b: &Board, persp: usize, out: &mut Accumulator) {
    let net = net();
    *out = net.feature_bias;
    let zone = persp_zone(b, persp);
    for sq in 0..64 {
        let pc = b.mailbox[sq];
        if pc != NO_PIECE {
            let w = &net.feature_weights[768 * zone + feature(persp, pc as usize, sq)].vals;
            for (v, d) in out.vals.iter_mut().zip(w) { *v += *d; }
        }
    }
}

/// Computes the child's accumulators from the parent's for `mv` played in position `b`.
pub fn update(b: &Board, mv: Move, parent: &AccPair, child: &mut AccPair,
    #[cfg(feature = "kb4nm")] after: &Board,
    #[cfg(feature = "kb4nm")] cache: &mut AccCache,
) {
    let (from, to) = (mv.from(), mv.to());
    let us = b.stm;
    let moved = b.mailbox[from] as usize;
    let placed = if mv.is_promo() { us * 6 + mv.promo_piece() } else { moved };

    let mut adds = [(0usize, 0usize); 2];
    let mut subs = [(0usize, 0usize); 2];
    let (mut na, mut ns) = (0, 0);
    adds[na] = (placed, to);
    na += 1;
    subs[ns] = (moved, from);
    ns += 1;
    if mv.is_capture() {
        let cap_sq = if mv.is_ep() { to ^ 8 } else { to };
        subs[ns] = (b.mailbox[cap_sq] as usize, cap_sq);
        ns += 1;
    } else if mv.is_castle() {
        let rook = us * 6 + ROOK;
        let (rf, rt) = if mv.flag() == Move::KING_CASTLE { (to + 1, to - 1) } else { (to - 2, to + 1) };
        adds[na] = (rook, rt);
        na += 1;
        subs[ns] = (rook, rf);
        ns += 1;
    }

    let net = net();
    for persp in 0..2 {
        #[cfg(feature = "kb4nm")]
        let offset = {
            let zone = persp_zone(b, persp);
            if persp == us && moved % 6 == KING && persp_zone(after, persp) != zone {
                if cfg!(feature = "kb-refresh-only") { refresh_one(after, persp, &mut child[persp]); }
                else { cache.refresh_one(after, persp, &mut child[persp]); }
                continue;
            }
            768 * zone
        };
        #[cfg(not(feature = "kb4nm"))]
        let offset = 0;
        let add: [&[i16; HIDDEN]; 2] = [
            &net.feature_weights[offset + feature(persp, adds[0].0, adds[0].1)].vals,
            &net.feature_weights[offset + feature(persp, adds[1].0, adds[1].1)].vals,
        ];
        let sub: [&[i16; HIDDEN]; 2] = [
            &net.feature_weights[offset + feature(persp, subs[0].0, subs[0].1)].vals,
            &net.feature_weights[offset + feature(persp, subs[1].0, subs[1].1)].vals,
        ];
        let src = &parent[persp].vals;
        let dst = &mut child[persp].vals;
        // Fused copy + update, specialised so each case is one vectorised pass.
        match (na, ns) {
            (1, 1) => {
                for i in 0..HIDDEN {
                    dst[i] = src[i] + add[0][i] - sub[0][i];
                }
            }
            (1, 2) => {
                for i in 0..HIDDEN {
                    dst[i] = src[i] + add[0][i] - sub[0][i] - sub[1][i];
                }
            }
            _ => {
                for i in 0..HIDDEN {
                    dst[i] = src[i] + add[0][i] + add[1][i] - sub[0][i] - sub[1][i];
                }
            }
        }
    }
}

/// One side's layer-1 inputs: round(clip(a) * clip(b) / 512) for the two halves of its accumulator.
#[cfg(target_feature = "avx2")]
#[inline(always)]
fn pairwise_inputs(v: &[i16; HIDDEN], out: &mut [u8]) {
    use std::arch::x86_64::*;
    const HALF: usize = HIDDEN / 2;
    unsafe {
        let zero = _mm256_setzero_si256();
        let qa = _mm256_set1_epi16(QA as i16);
        let clip = |p: *const i16| _mm256_min_epi16(_mm256_max_epi16(_mm256_load_si256(p as *const __m256i), zero), qa);
        let mut i = 0;
        while i < HALF {
            // mulhrs((a << 6), b) = (a * b * 64 + 2^14) >> 15 = round(a * b / 512)
            let r0 = _mm256_mulhrs_epi16(_mm256_slli_epi16::<6>(clip(v.as_ptr().add(i))), clip(v.as_ptr().add(i + HALF)));
            let r1 = _mm256_mulhrs_epi16(_mm256_slli_epi16::<6>(clip(v.as_ptr().add(i + 16))), clip(v.as_ptr().add(i + HALF + 16)));
            // packus interleaves the 128-bit lanes; the permute puts them back in order.
            let packed = _mm256_permute4x64_epi64::<0b11_01_10_00>(_mm256_packus_epi16(r0, r1));
            _mm256_storeu_si256(out.as_mut_ptr().add(i) as *mut __m256i, packed);
            i += 32;
        }
    }
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
fn pairwise_inputs(v: &[i16; HIDDEN], out: &mut [u8]) {
    use std::arch::aarch64::*;
    const HALF: usize = HIDDEN / 2;
    unsafe {
        let zero = vdupq_n_s16(0);
        let qa = vdupq_n_s16(QA as i16);
        let clip = |p: *const i16| vminq_s16(vmaxq_s16(vld1q_s16(p), zero), qa);
        let mut i = 0;
        while i < HALF {
            // vqrdmulh(a << 6, b) = (a * b * 64 * 2 + 2^15) >> 16 = round(a * b / 512), as mulhrs on x86.
            let r = vqrdmulhq_s16(vshlq_n_s16::<6>(clip(v.as_ptr().add(i))), clip(v.as_ptr().add(i + HALF)));
            vst1_u8(out.as_mut_ptr().add(i), vqmovun_s16(r));
            i += 8;
        }
    }
}

#[cfg(not(any(target_feature = "avx2", target_arch = "aarch64")))]
#[inline(always)]
fn pairwise_inputs(v: &[i16; HIDDEN], out: &mut [u8]) {
    const HALF: usize = HIDDEN / 2;
    for i in 0..HALF {
        let a = v[i].clamp(0, QA as i16) as u32;
        let b = v[i + HALF].clamp(0, QA as i16) as u32;
        out[i] = ((a * b + 256) >> 9) as u8;
    }
}

/// Dot product of 0..=127 inputs with i8 weights (HIDDEN long).
#[cfg(target_feature = "avx2")]
#[inline(always)]
fn dot_u8_i8(x: &[u8; HIDDEN], w: &[i8]) -> i32 {
    use std::arch::x86_64::*;
    unsafe {
        let ones = _mm256_set1_epi16(1);
        let mut sum = _mm256_setzero_si256();
        let mut i = 0;
        while i < HIDDEN {
            let xv = _mm256_load_si256(x.as_ptr().add(i) as *const __m256i);
            let wv = _mm256_loadu_si256(w.as_ptr().add(i) as *const __m256i);
            // u8 * i8 pairs summed into i16 (at most 2 * 127 * 127, no saturation), then into i32.
            sum = _mm256_add_epi32(sum, _mm256_madd_epi16(_mm256_maddubs_epi16(xv, wv), ones));
            i += 32;
        }
        let s = _mm_add_epi32(_mm256_castsi256_si128(sum), _mm256_extracti128_si256::<1>(sum));
        let s = _mm_add_epi32(s, _mm_shuffle_epi32::<0b01_00_11_10>(s));
        let s = _mm_add_epi32(s, _mm_shuffle_epi32::<0b10_11_00_01>(s));
        _mm_cvtsi128_si32(s)
    }
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
fn dot_u8_i8(x: &[u8; HIDDEN], w: &[i8]) -> i32 {
    use std::arch::aarch64::*;
    // The inputs are 0..=127, so they are also valid i8. Four independent sums so the multiply-accumulates
    // don't wait on each other.
    unsafe {
        let mut s = [vdupq_n_s32(0); 4];
        let mut i = 0;
        while i < HIDDEN {
            for (k, sk) in s.iter_mut().enumerate() {
                let xv = vreinterpretq_s8_u8(vld1q_u8(x.as_ptr().add(i + 16 * k)));
                let wv = vld1q_s8(w.as_ptr().add(i + 16 * k));
                // sdot: four i8 products summed into each i32 lane (ARMv8.2 dot product); otherwise widening
                // multiplies pairwise-added into i32.
                #[cfg(target_feature = "dotprod")]
                {
                    *sk = vdotq_s32(*sk, xv, wv);
                }
                #[cfg(not(target_feature = "dotprod"))]
                {
                    *sk = vpadalq_s16(*sk, vmull_s8(vget_low_s8(xv), vget_low_s8(wv)));
                    *sk = vpadalq_s16(*sk, vmull_high_s8(xv, wv));
                }
            }
            i += 64;
        }
        vaddvq_s32(vaddq_s32(vaddq_s32(s[0], s[1]), vaddq_s32(s[2], s[3])))
    }
}

#[cfg(not(any(target_feature = "avx2", target_arch = "aarch64")))]
#[inline(always)]
fn dot_u8_i8(x: &[u8; HIDDEN], w: &[i8]) -> i32 {
    x.iter().zip(w).map(|(&a, &b)| a as i32 * b as i32).sum()
}

/// Evaluation in centipawns from the side to move's point of view.
/// `pieces` = number of pieces on the board (both colors, kings included), which picks the bucket.
pub fn evaluate(acc: &AccPair, stm: usize, pieces: u32) -> i32 {
    let net = net();
    // Same rule as bullet's MaterialCount<N>: (pieces - 2) / ceil(32 / N).
    let bucket = ((pieces.saturating_sub(2)) as usize / 32usize.div_ceil(OUTPUT_BUCKETS)).min(OUTPUT_BUCKETS - 1);
    const HALF: usize = HIDDEN / 2;
    // Layer 1 in 8-bit integers: each side's clipped first-layer outputs, the first half times the second
    // half (side to move first), scaled to 0..=127, dotted with the i8 weights of this bucket's 16 outputs.
    #[repr(align(32))]
    struct Inputs([u8; HIDDEN]);
    let mut x = Inputs([0; HIDDEN]);
    for (side, base) in [(stm, 0), (stm ^ 1, HALF)] {
        pairwise_inputs(&acc[side].vals, &mut x.0[base..base + HALF]);
    }
    let q = unsafe { L1Q.unwrap_unchecked() };
    let mut h1 = net.l1b[bucket];
    for (o, h) in h1.iter_mut().enumerate() {
        let row = &q.w[(bucket * L1 + o) * HIDDEN..(bucket * L1 + o + 1) * HIDDEN];
        *h += dot_u8_i8(&x.0, row) as f32 * q.mul[bucket * L1 + o];
    }
    let mut h2 = net.l2b[bucket];
    for (j, &h) in h1.iter().enumerate() {
        let x = h.clamp(0.0, 1.0);
        let x = x * x;
        let w = &net.l2w[j][bucket];
        for o in 0..L2 {
            h2[o] += w[o] * x;
        }
    }
    let mut out = net.l3b[bucket];
    for (k, &h) in h2.iter().enumerate() {
        let x = h.clamp(0.0, 1.0);
        out += net.l3w[k][bucket] * x * x;
    }
    (out * SCALE) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attacks;

    /// Incremental updates must exactly match a from-scratch refresh after every kind of move.
    #[test]
    fn incremental_matches_refresh() {
        attacks::init();
        let mut seed = 0x1234_5678_9abc_def1u64;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut bytes = vec![0u8; NET_FILE_BYTES];
        for v in bytes[..(768 * INPUT_BUCKETS * HIDDEN + HIDDEN) * 2].chunks_exact_mut(2) {
            v.copy_from_slice(&((rand() % 101) as i16 - 50).to_le_bytes());
        }
        assert!(from_bytes(&bytes[..bytes.len()-1]).is_err());
        assert!(validate_size(if cfg!(feature="kb4nm") { 2_118_208 } else { 6_836_800 }).is_err());
        set_net(from_bytes(&bytes).unwrap());

        let fens = [
            crate::board::START_FEN,
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
            "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1",
            "r3k2r/8/8/8/8/8/8/R3K2R b KQkq - 0 1",
            "4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1",
            "4k3/P7/8/8/8/8/7p/4K3 w - - 0 1",
            "4k3/P7/8/8/8/8/7p/4K3 b - - 0 1",
        ];
        let mut checked = 0;
        let (mut castles, mut eps, mut promos, mut nulls) = (0,0,0,0);
        #[cfg(feature = "kb4nm")]
        let mut cache = AccCache::default();
        #[cfg(feature = "kb4nm")]
        let mut crossings = 0;
        for fen in fens {
            for _ in 0..40 {
                let mut b = Board::from_fen(fen).unwrap();
                let mut acc = [Accumulator { vals: [0; HIDDEN] }; 2];
                refresh(&b, &mut acc);
                for _ in 0..120 {
                    let moves = b.legal_moves();
                    if moves.is_empty() {
                        break;
                    }
                    // Revisit the same parent after sibling updates, including forced special moves.
                    if checked % 37 == 0 || checked == 0 {
                        for mv in moves.iter().copied() {
                            let mut after = b; after.make_move(mv);
                            let mut next = acc;
                            update(&b, mv, &acc, &mut next,
                                #[cfg(feature = "kb4nm")] &after,
                                #[cfg(feature = "kb4nm")] &mut cache,
                            );
                            let mut fresh = acc; refresh(&after, &mut fresh);
                            for p in 0..2 {assert_eq!(next[p].vals, fresh[p].vals);}
                            castles += usize::from(mv.is_castle()); eps += usize::from(mv.is_ep()); promos += usize::from(mv.is_promo());
                        }
                    }
                    if checked % 17 == 0 && !b.in_check() {
                        let mut nb=b; nb.make_null();
                        let mut fresh=acc; refresh(&nb,&mut fresh);
                        for p in 0..2 {assert_eq!(acc[p].vals,fresh[p].vals);}
                        if let Some(nmv)=nb.legal_moves().first().copied() {
                            let mut after=nb;after.make_move(nmv);let mut next=acc;
                            update(&nb,nmv,&acc,&mut next,
                                #[cfg(feature = "kb4nm")] &after,
                                #[cfg(feature = "kb4nm")] &mut cache,
                            );
                            refresh(&after,&mut fresh);for p in 0..2 {assert_eq!(next[p].vals,fresh[p].vals);}
                        }
                        nulls += 1;
                    }
                    let mv = moves[(rand() % moves.len() as u64) as usize];
                    let mut next = [Accumulator { vals: [0; HIDDEN] }; 2];
                    let mut after=b; after.make_move(mv);
                    #[cfg(feature = "kb4nm")]
                    { crossings += usize::from(persp_zone(&b,b.stm)!=persp_zone(&after,b.stm)); }
                    update(&b, mv, &acc, &mut next,
                        #[cfg(feature = "kb4nm")] &after,
                        #[cfg(feature = "kb4nm")] &mut cache,
                    );
                    castles += usize::from(mv.is_castle()); eps += usize::from(mv.is_ep()); promos += usize::from(mv.is_promo());
                    b.make_move(mv);
                    let mut fresh = [Accumulator { vals: [0; HIDDEN] }; 2];
                    refresh(&b, &mut fresh);
                    for p in 0..2 {
                        assert_eq!(next[p].vals, fresh[p].vals, "mismatch after {mv} in {}", b.to_fen());
                    }
                    acc = next;
                    checked += 1;
                }
            }
        }
        assert!(checked > 10_000);
        assert!(castles>0 && eps>0 && promos>0 && nulls>0);
        println!("{checked} random moves plus sibling revisits; castles {castles}, EP {eps}, promotions {promos}, nulls {nulls}");
        #[cfg(feature = "kb4nm")]
        {
            assert!(crossings>100);
            // Visit every king square from both colours and reuse cache entries across unrelated boards.
            let mut boundary_moves=0;
            for side in [WHITE,BLACK] {for king in 0usize..64 {
                let far=|s:usize| (s%8).abs_diff(king%8).max((s/8).abs_diff(king/8))>=3;
                let other=if far(0){0}else{63};let mut fen=String::new();
                for rank in (0..8).rev(){let mut empty=0;for file in 0..8 {
                    let sq=rank*8+file;let pc=if sq==king{if side==WHITE{'K'}else{'k'}}else if sq==other{if side==WHITE{'k'}else{'K'}}else{' '};
                    if pc==' '{empty+=1;}else{if empty>0{fen.push(char::from_digit(empty,10).unwrap());empty=0;}fen.push(pc);}
                }if empty>0{fen.push(char::from_digit(empty,10).unwrap());}if rank>0{fen.push('/');}}
                fen+=&format!(" {} - - 0 1",if side==WHITE{"w"}else{"b"});
                let b=Board::from_fen(&fen).unwrap();let mut acc=[Accumulator{vals:[0;HIDDEN]};2];refresh(&b,&mut acc);
                for mv in b.legal_moves(){let mut after=b;after.make_move(mv);let mut next=acc;
                    update(&b,mv,&acc,&mut next,&after,&mut cache);let mut fresh=acc;refresh(&after,&mut fresh);
                    for p in 0..2 {assert_eq!(next[p].vals,fresh[p].vals);}boundary_moves+=1;
                }
            }}
            // Stale cache entries must be discarded when any new network becomes active.
            let b=Board::startpos();let mut cached=[Accumulator{vals:[0;HIDDEN]};2];
            for (p,a) in cached.iter_mut().enumerate(){cache.refresh_one(&b,p,a);}
            for v in bytes[..(768 * INPUT_BUCKETS * HIDDEN + HIDDEN)*2].chunks_exact_mut(2){let x=i16::from_le_bytes(v.try_into().unwrap());v.copy_from_slice(&(-x+1).to_le_bytes());}
            set_net(from_bytes(&bytes).unwrap());let mut fresh=cached;refresh(&b,&mut fresh);
            assert_ne!(cached[0].vals,fresh[0].vals);
            for (p,a) in cached.iter_mut().enumerate(){cache.refresh_one(&b,p,a);assert_eq!(a.vals,fresh[p].vals);}
            println!("zone crossings {crossings}, exhaustive king moves {boundary_moves}, cache generation invalidation passed");
        }
    }
}

/// Full evaluation from scratch (for the "eval" command and tests).
pub fn evaluate_board(b: &Board) -> i32 {
    let mut acc = [Accumulator { vals: [0; HIDDEN] }; 2];
    refresh(b, &mut acc);
    evaluate(&acc, b.stm, b.occ().count_ones())
}
