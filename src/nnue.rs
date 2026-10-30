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
const _: () = assert!(HIDDEN % 128 == 0 && L1 == 16);
const SCALE: f32 = 400.0;
const QA: i32 = 255;

/// Size of bullet's quantised output (it may be padded to a multiple of 64).
pub const NET_BYTES: usize = (768 * HIDDEN + HIDDEN) * 2
    + (HIDDEN * OUTPUT_BUCKETS * L1 + OUTPUT_BUCKETS * L1 + L1 * OUTPUT_BUCKETS * L2 + OUTPUT_BUCKETS * L2 + L2 * OUTPUT_BUCKETS + OUTPUT_BUCKETS) * 4;

#[derive(Clone, Copy)]
#[repr(C, align(64))]
pub struct Accumulator {
    pub vals: [i16; HIDDEN],
}

#[repr(C)]
pub struct Network {
    feature_weights: [Accumulator; 768],
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

/// Sixteen outputs by four inputs, aligned so both AVX2 loads stay in one cache line.
#[derive(Clone, Copy)]
#[repr(C, align(64))]
struct L1Block([i8; 64]);

/// Quantised L1 with dense VNNI rows and sparse input-major blocks.
struct L1Quant {
    w: Vec<i8>,
    /// [bucket][four-input block][output][four weights]. Runtime-only repacking.
    blocks: Vec<L1Block>,
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
    let blocks = pack_l1(&w);
    unsafe {
        L1Q = Some(Box::leak(Box::new(L1Quant { w, blocks, mul })));
        NET = Some(net);
    }
}

fn pack_l1(w: &[i8]) -> Vec<L1Block> {
    let mut blocks = vec![L1Block([0; 64]); w.len() / 64];
    for b in 0..OUTPUT_BUCKETS {
        for block in 0..HIDDEN / 4 {
            for o in 0..L1 {
                for j in 0..4 {
                    blocks[b * (HIDDEN / 4) + block].0[o * 4 + j] =
                        w[(b * L1 + o) * HIDDEN + block * 4 + j];
                }
            }
        }
    }
    blocks
}

fn from_bytes(bytes: &[u8]) -> Result<&'static Network, String> {
    if bytes.len() < NET_BYTES {
        return Err(format!("network file is {} bytes, expected at least {NET_BYTES}", bytes.len()));
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
    if let Ok(net) = from_bytes(DEFAULT_NET) {
        set_net(net);
    }
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
    crate::profile_hit!(0);
    let net = net();
    for (persp, a) in acc.iter_mut().enumerate() {
        *a = net.feature_bias;
        for sq in 0..64 {
            let pc = b.mailbox[sq];
            if pc != NO_PIECE {
                let w = &net.feature_weights[feature(persp, pc as usize, sq)].vals;
                for (v, d) in a.vals.iter_mut().zip(w) {
                    *v += *d;
                }
            }
        }
    }
}

/// Computes the child's accumulators from the parent's for `mv` played in position `b`.
pub fn update(b: &Board, mv: Move, parent: &AccPair, child: &mut AccPair) {
    crate::profile_hit!(if mv.is_promo() { 4 } else if mv.is_castle() { 3 } else if mv.is_capture() { 2 } else { 1 });
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
        let add: [&[i16; HIDDEN]; 2] = [
            &net.feature_weights[feature(persp, adds[0].0, adds[0].1)].vals,
            &net.feature_weights[feature(persp, adds[1].0, adds[1].1)].vals,
        ];
        let sub: [&[i16; HIDDEN]; 2] = [
            &net.feature_weights[feature(persp, subs[0].0, subs[0].1)].vals,
            &net.feature_weights[feature(persp, subs[1].0, subs[1].1)].vals,
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
    crate::profile_hit!(5);
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
    crate::profile_hit!(5);
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
    crate::profile_hit!(5);
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
        #[cfg(not(target_feature = "avxvnni"))]
        let ones = _mm256_set1_epi16(1);
        let mut sums = [_mm256_setzero_si256(); 4];
        let mut i = 0;
        while i < HIDDEN {
            for (k, sum) in sums.iter_mut().enumerate() {
                let xv = _mm256_load_si256(x.as_ptr().add(i + 32 * k) as *const __m256i);
                let wv = _mm256_loadu_si256(w.as_ptr().add(i + 32 * k) as *const __m256i);
                #[cfg(target_feature = "avxvnni")]
                { *sum = _mm256_dpbusd_avx_epi32(*sum, xv, wv); }
                #[cfg(not(target_feature = "avxvnni"))]
                {
                    // Inputs <=127: pair sums fit i16, so maddubs never saturates.
                    *sum = _mm256_add_epi32(*sum, _mm256_madd_epi16(_mm256_maddubs_epi16(xv, wv), ones));
                }
            }
            i += 128;
        }
        let sum = _mm256_add_epi32(_mm256_add_epi32(sums[0], sums[1]), _mm256_add_epi32(sums[2], sums[3]));
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
                for offset in [0, 16] {
                    let xv = vreinterpretq_s8_u8(vld1q_u8(x.as_ptr().add(i + 32 * k + offset)));
                    let wv = vld1q_s8(w.as_ptr().add(i + 32 * k + offset));
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
            }
            i += 128;
        }
        vaddvq_s32(vaddq_s32(vaddq_s32(s[0], s[1]), vaddq_s32(s[2], s[3])))
    }
}

#[cfg(not(any(target_feature = "avx2", target_arch = "aarch64")))]
#[inline(always)]
fn dot_u8_i8(x: &[u8; HIDDEN], w: &[i8]) -> i32 {
    let mut sums = [0i32; 4];
    for i in (0..HIDDEN).step_by(128) {
        for (k, sum) in sums.iter_mut().enumerate() {
            for j in 0..32 {
                let index = i + 32 * k + j;
                *sum += x[index] as i32 * w[index] as i32;
            }
        }
    }
    (sums[0] + sums[1]) + (sums[2] + sums[3])
}

struct Nonzero8 { len: u8, offsets: [u16; 8] }
const NONZERO8: [Nonzero8; 256] = {
    let mut table = [const { Nonzero8 { len: 0, offsets: [0; 8] } }; 256];
    let mut mask = 0;
    while mask < 256 {
        let mut bit = 0;
        while bit < 8 {
            if mask & (1 << bit) != 0 {
                let n = table[mask].len as usize;
                table[mask].offsets[n] = bit as u16;
                table[mask].len += 1;
            }
            bit += 1;
        }
        mask += 1;
    }
    table
};

/// Each surviving group broadcasts four inputs into 16 output dot products.
/// Pair sums fit i16; complete sums fit i32. Only integer addition is reordered.
#[cfg(target_feature = "avx2")]
fn sparse_l1(x: &[u8; HIDDEN], weights: &[L1Block]) -> [i32; L1] {
    crate::profile_hit!(7);
    use std::arch::x86_64::*;
    debug_assert_eq!(weights.len(), HIDDEN / 4);
    unsafe {
        let mut lo = _mm256_setzero_si256();
        let mut hi = _mm256_setzero_si256();
        let ones = _mm256_set1_epi16(1);
        let mut blocks = [0u16; HIDDEN / 4 + 8];
        let mut count = 0;
        for base in (0..HIDDEN / 4).step_by(8) {
            let input = _mm256_loadu_si256(x.as_ptr().add(base * 4) as *const __m256i);
            // Every byte <=127, hence each nonzero four-byte word is positive.
            let mask = _mm256_movemask_ps(_mm256_castsi256_ps(_mm256_cmpgt_epi32(input, _mm256_setzero_si256()))) as usize;
            let indices = &NONZERO8[mask];
            let offsets = _mm_loadu_si128(indices.offsets.as_ptr() as *const __m128i);
            _mm_storeu_si128(blocks.as_mut_ptr().add(count) as *mut __m128i,
                _mm_add_epi16(offsets, _mm_set1_epi16(base as i16)));
            count += indices.len as usize;
        }
        // One long loop avoids an unpredictable short loop for each eight-group mask.
        for &block in &blocks[..count] {
        let block = block as usize;
        let word = std::ptr::read_unaligned(x.as_ptr().add(block * 4) as *const i32);
        let broadcast = _mm256_set1_epi32(word);
        let w = weights.as_ptr().add(block) as *const i8;
        let wl = _mm256_load_si256(w as *const __m256i);
        let wh = _mm256_load_si256(w.add(32) as *const __m256i);
        lo = _mm256_add_epi32(lo, _mm256_madd_epi16(_mm256_maddubs_epi16(broadcast, wl), ones));
        hi = _mm256_add_epi32(hi, _mm256_madd_epi16(_mm256_maddubs_epi16(broadcast, wh), ones));
    }
    let mut out = [0i32; L1];
    _mm256_storeu_si256(out.as_mut_ptr() as *mut __m256i, lo);
    _mm256_storeu_si256(out.as_mut_ptr().add(8) as *mut __m256i, hi);
    out
}
}

#[cfg(target_arch = "aarch64")]
fn sparse_l1(x: &[u8; HIDDEN], weights: &[L1Block]) -> [i32; L1] {
    crate::profile_hit!(7);
use std::arch::aarch64::*;
debug_assert_eq!(weights.len(), HIDDEN / 4);
unsafe {
    let mut sums = [vdupq_n_s32(0); 4];
    let bits = [1u32, 2, 4, 8];
    let mut blocks = [0u16; HIDDEN / 4 + 8];
    let mut count = 0;
    for base in (0..HIDDEN / 4).step_by(8) {
        let a = vld1q_u32(x.as_ptr().add(base * 4) as *const u32);
        let b = vld1q_u32(x.as_ptr().add(base * 4 + 16) as *const u32);
        let a = vandq_u32(vcgtq_u32(a, vdupq_n_u32(0)), vld1q_u32(bits.as_ptr()));
        let b = vandq_u32(vcgtq_u32(b, vdupq_n_u32(0)), vld1q_u32(bits.as_ptr()));
        let mask = (vaddvq_u32(a) | (vaddvq_u32(b) << 4)) as usize;
        let indices = &NONZERO8[mask];
        let offsets = vld1q_u16(indices.offsets.as_ptr());
        vst1q_u16(blocks.as_mut_ptr().add(count), vaddq_u16(offsets, vdupq_n_u16(base as u16)));
        count += indices.len as usize;
    }
    for &block in &blocks[..count] {
        let block = block as usize;
        let word = std::ptr::read_unaligned(x.as_ptr().add(block * 4) as *const i32);
        let broadcast = vreinterpretq_s8_s32(vdupq_n_s32(word));
        for (k, sum) in sums.iter_mut().enumerate() {
            let w = vld1q_s8((weights.as_ptr().add(block) as *const i8).add(k * 16));
            #[cfg(target_feature = "dotprod")]
            { *sum = vdotq_s32(*sum, broadcast, w); }
            #[cfg(not(target_feature = "dotprod"))]
            {
                let lo = vpaddlq_s16(vmull_s8(vget_low_s8(broadcast), vget_low_s8(w)));
                let hi = vpaddlq_s16(vmull_high_s8(broadcast, w));
                *sum = vaddq_s32(*sum, vpaddq_s32(lo, hi));
            }
        }
    }
    let mut out = [0i32; L1];
    for (k, sum) in sums.iter().enumerate() { vst1q_s32(out.as_mut_ptr().add(k * 4), *sum); }
    out
}
}

#[cfg(not(any(target_feature = "avx2", target_arch = "aarch64")))]
fn sparse_l1(x: &[u8; HIDDEN], weights: &[L1Block]) -> [i32; L1] {
    crate::profile_hit!(7);
let mut sums = [0i32; L1];
let mut blocks = [0u16; HIDDEN / 4 + 8];
let mut count = 0;
for base in (0..HIDDEN / 4).step_by(8) {
    let mut mask = 0;
    for offset in 0..8 {
        if x[(base + offset) * 4..(base + offset + 1) * 4].iter().any(|&v| v != 0) { mask |= 1 << offset; }
    }
    let indices = &NONZERO8[mask];
    for k in 0..8 { blocks[count + k] = base as u16 + indices.offsets[k]; }
    count += indices.len as usize;
}
for &block in &blocks[..count] {
        let block = block as usize;
        for (o, sum) in sums.iter_mut().enumerate() {
            for j in 0..4 { *sum += x[block * 4 + j] as i32 * weights[block].0[o * 4 + j] as i32; }
        }
    }
    sums
}

#[repr(align(32))]
struct Inputs([u8; HIDDEN]);

#[inline(always)]
fn inputs(acc: &AccPair, stm: usize) -> Inputs {
    const HALF: usize = HIDDEN / 2;
    // Layer 1 in 8-bit integers: each side's clipped first-layer outputs, the first half times the second
    // half (side to move first), scaled to 0..=127, dotted with the i8 weights of this bucket's 16 outputs.
    let mut x = Inputs([0; HIDDEN]);
    for (side, base) in [(stm, 0), (stm ^ 1, HALF)] {
        pairwise_inputs(&acc[side].vals, &mut x.0[base..base + HALF]);
    }
    x
}

#[inline(always)]
fn dense_l1(x: &[u8; HIDDEN], bucket: usize) -> [i32; L1] {
    crate::profile_hit!(6);
    let q = unsafe { L1Q.unwrap_unchecked() };
    std::array::from_fn(|o| dot_u8_i8(x, &q.w[(bucket * L1 + o) * HIDDEN..(bucket * L1 + o + 1) * HIDDEN]))
}

/// Evaluation in centipawns from the side to move's point of view.
/// `pieces` = number of pieces on the board (both colors, kings included), which picks the bucket.
pub fn evaluate(acc: &AccPair, stm: usize, pieces: u32) -> i32 {
    crate::profile_hit!(9);
    // Same rule as bullet's MaterialCount<N>: (pieces - 2) / ceil(32 / N).
    let bucket = ((pieces.saturating_sub(2)) as usize / 32usize.div_ceil(OUTPUT_BUCKETS)).min(OUTPUT_BUCKETS - 1);
    let x = inputs(acc, stm);
    let q = unsafe { L1Q.unwrap_unchecked() };
    // VNNI and ARM dotprod keep the dense path (sparse measured ~1% slower on the Neoverse N1 bot server);
    // AVX2, other NEON and scalar use the sparse blocks. Both give identical integers.
    let dots: [i32; L1] = if cfg!(any(target_feature = "avxvnni", all(target_arch = "aarch64", target_feature = "dotprod"))) {
        dense_l1(&x.0, bucket)
    } else {
        sparse_l1(&x.0, &q.blocks[bucket * (HIDDEN / 4)..(bucket + 1) * (HIDDEN / 4)])
    };
    tail(dots, bucket)
}

#[inline(always)]
fn tail(dots: [i32; L1], bucket: usize) -> i32 {
    crate::profile_hit!(8);
    (tail_value(dots, bucket) * SCALE) as i32
}

#[inline(always)]
fn tail_value(dots: [i32; L1], bucket: usize) -> f32 {
    let net = net();
    let q = unsafe { L1Q.unwrap_unchecked() };
    let mut h1 = net.l1b[bucket];
    for (o, h) in h1.iter_mut().enumerate() {
        *h += dots[o] as f32 * q.mul[bucket * L1 + o];
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
    out
}

pub(crate) mod profiling {
    use super::*;
    use crate::profile::{time, NAMES};
    use std::hint::black_box;

    pub fn measure(n: usize, boards: &[Board], moves: &[(Board, Move)]) -> [f64; 15] {
        let mut costs = [0.0; 15];
        let zero = [Accumulator { vals: [0; HIDDEN] }; 2];
        // Include children to exercise buckets, both side orders, and realistic activation sparsity.
        let mut corpus = boards.to_vec();
        for &(mut b, mv) in moves { b.make_move(mv); corpus.push(b); }
        let accs: Vec<_> = corpus.iter().map(|b| { let mut acc = zero; refresh(b, &mut acc); acc }).collect();
        let xs: Vec<_> = corpus.iter().zip(&accs).map(|(b, acc)| inputs(acc, b.stm)).collect();
        let buckets: Vec<_> = corpus.iter().map(|b| ((b.occ().count_ones().saturating_sub(2) / 4) as usize).min(7)).collect();
        let q = unsafe { L1Q.unwrap_unchecked() };
        let dots: Vec<_> = xs.iter().zip(&buckets).map(|(x, &b)| dense_l1(&x.0, b)).collect();
        // Fail immediately on an integer kernel mismatch; print a portable eval checksum.
        let mut digest = 0xcbf29ce484222325u64;
        for i in 0..corpus.len() {
            let b = buckets[i];
            assert_eq!(dots[i], sparse_l1(&xs[i].0, &q.blocks[b * (HIDDEN / 4)..(b + 1) * (HIDDEN / 4)]));
            let v = evaluate(&accs[i], corpus[i].stm, corpus[i].occ().count_ones());
            digest = (digest ^ v as u32 as u64).wrapping_mul(0x100000001b3);
            digest = (digest ^ tail_value(dots[i], b).to_bits() as u64).wrapping_mul(0x100000001b3);
        }
        println!("corpus positions={} eval_digest={digest:016x} dense_sparse=equal", corpus.len());
        let mut dst = zero;
        costs[0] = time(NAMES[0], n, |i| { refresh(black_box(&corpus[i % corpus.len()]), black_box(&mut dst)); black_box(&dst); });
        for kind in 1..=4 {
            let selected: Vec<_> = moves.iter().filter(|(_, mv)| {
                (if mv.is_promo() { 4 } else if mv.is_castle() { 3 } else if mv.is_capture() { 2 } else { 1 }) == kind
            }).map(|&(b, mv)| { let mut acc = zero; refresh(&b, &mut acc); (b, mv, acc) }).collect();
            assert!(!selected.is_empty(), "missing update fixture {kind}");
            println!("fixtures {}={}", NAMES[kind], selected.len());
            costs[kind] = time(NAMES[kind], n, |i| { let (b, mv, acc) = black_box(&selected[i % selected.len()]); update(b, *mv, acc, black_box(&mut dst)); black_box(&dst); });
        }
        let mut pair = [0u8; HIDDEN / 2];
        costs[5] = time(NAMES[5], n, |i| { pairwise_inputs(black_box(&accs[i % accs.len()][i % 2].vals), black_box(&mut pair)); black_box(&pair); });
        costs[6] = time(NAMES[6], n, |i| dense_l1(black_box(&xs[i % xs.len()].0), black_box(buckets[i % buckets.len()])));
        costs[7] = time(NAMES[7], n, |i| { let b = buckets[i % buckets.len()]; sparse_l1(black_box(&xs[i % xs.len()].0), black_box(&q.blocks[b * (HIDDEN / 4)..(b + 1) * (HIDDEN / 4)])) });
        costs[8] = time(NAMES[8], n, |i| tail(black_box(dots[i % dots.len()]), black_box(buckets[i % buckets.len()])));
        costs[9] = time(NAMES[9], n, |i| { let j = i % accs.len(); evaluate(black_box(&accs[j]), black_box(corpus[j].stm), black_box(corpus[j].occ().count_ones())) });
        costs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attacks;

    #[test]
    fn pairwise_matches_scalar_rounding() {
        let mut v = Accumulator { vals: [0; HIDDEN] };
        let mut out = [0; HIDDEN / 2];
        for trial in 0..256 {
            for i in 0..HIDDEN { v.vals[i] = ((i * 109 + trial * 251) & 65535) as i16; }
            pairwise_inputs(&v.vals, &mut out);
            for i in 0..HIDDEN / 2 {
                let a = v.vals[i].clamp(0, 255) as i32;
                let b = v.vals[i + HIDDEN / 2].clamp(0, 255) as i32;
                assert_eq!(out[i], ((a * b + 256) >> 9) as u8);
            }
        }
    }

    // Keep the original serialized-layout arithmetic as a bitwise f32 oracle.
    #[test]
    fn tail_preserves_float_bits() {
        let mut seed = 0x9321_bfab_fdae_2381u64;
        let mut rand = || { seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; seed };
        let mut bytes = vec![0u8; NET_BYTES];
        let float_start = (768 * HIDDEN + HIDDEN) * 2;
        for chunk in bytes[float_start..].chunks_exact_mut(4) {
            let v = ((rand() % 20001) as f32 - 10000.0) * 0.00001;
            chunk.copy_from_slice(&v.to_le_bytes());
        }
        set_net(from_bytes(&bytes).unwrap());
        let net = net();
        let q = unsafe { L1Q.unwrap_unchecked() };
        for trial in 0..512 {
            let bucket = trial % OUTPUT_BUCKETS;
            let dots: [i32; L1] = std::array::from_fn(|_| (rand() % 2_000_001) as i32 - 1_000_000);
            let mut h1 = net.l1b[bucket];
            for (o, h) in h1.iter_mut().enumerate() { *h += dots[o] as f32 * q.mul[bucket * L1 + o]; }
            let mut h2 = net.l2b[bucket];
            for (j, &h) in h1.iter().enumerate() {
                let x = h.clamp(0.0, 1.0); let x = x * x;
                for o in 0..L2 { h2[o] += net.l2w[j][bucket][o] * x; }
            }
            let mut expected = net.l3b[bucket];
            for (k, &h) in h2.iter().enumerate() { let x = h.clamp(0.0, 1.0); expected += net.l3w[k][bucket] * x * x; }
            assert_eq!(tail_value(dots, bucket).to_bits(), expected.to_bits(), "trial {trial}");
        }
    }

    #[test]
    fn sparse_blocks_match_dense_dots() {
        let mut seed = 0xa432_2817_4521_789bu64;
        let mut rand = || { seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; seed };
        let w: Vec<i8> = (0..OUTPUT_BUCKETS * L1 * HIDDEN).map(|_| rand() as i8).collect();
        let packed = pack_l1(&w);
        #[repr(align(32))]
        struct Inputs([u8; HIDDEN]);
        for trial in 0..320 {
            let mut x = Inputs([0; HIDDEN]);
            for i in 0..HIDDEN {
                x.0[i] = if trial < 256 {
                    if trial & (1 << ((i / 4) % 8)) != 0 { 127 } else { 0 }
                } else if trial == 256 {
                    127
                } else {
                    (rand() & 127) as u8
                };
            }
            let b = trial % OUTPUT_BUCKETS;
            let actual = sparse_l1(&x.0, &packed[b * (HIDDEN / 4)..(b + 1) * (HIDDEN / 4)]);
            for o in 0..L1 {
                let row = &w[(b * L1 + o) * HIDDEN..(b * L1 + o + 1) * HIDDEN];
                let expected: i32 = x.0.iter().zip(row).map(|(&a, &b)| a as i32 * b as i32).sum();
                assert_eq!(actual[o], expected, "trial {trial}, bucket {b}, output {o}");
            }
        }
    }

    #[test]
    fn integer_dot_matches_scalar_oracle() {
        #[repr(align(32))]
        struct Inputs([u8; HIDDEN]);
        let mut x = Inputs([0; HIDDEN]);
        let mut w = [0i8; HIDDEN];
        let mut seed = 0x9351_1234_8765_abceu64;
        for trial in 0..1024 {
            for i in 0..HIDDEN {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                x.0[i] = if trial < 4 { 127 } else { (seed & 127) as u8 };
                w[i] = match trial {
                    0 => -128, 1 => 127, 2 => if i % 2 == 0 { -128 } else { 127 },
                    _ => (seed >> 32) as i8,
                };
            }
            let expected: i32 = x.0.iter().zip(&w).map(|(&a, &b)| a as i32 * b as i32).sum();
            assert_eq!(dot_u8_i8(&x.0, &w), expected, "trial {trial}");
        }
    }

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
        let bytes: Vec<u8> = (0..NET_BYTES / 2).flat_map(|_| ((rand() % 101) as i16 - 50).to_le_bytes()).collect();
        set_net(from_bytes(&bytes).unwrap());

        let fens = [
            crate::board::START_FEN,
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
            "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
        ];
        let mut checked = 0;
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
                    let mv = moves[(rand() % moves.len() as u64) as usize];
                    let mut next = [Accumulator { vals: [0; HIDDEN] }; 2];
                    update(&b, mv, &acc, &mut next);
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
    }
}

/// Full evaluation from scratch (for the "eval" command and tests).
pub fn evaluate_board(b: &Board) -> i32 {
    let mut acc = [Accumulator { vals: [0; HIDDEN] }; 2];
    refresh(b, &mut acc);
    evaluate(&acc, b.stm, b.occ().count_ones())
}
