# Dual activation L2 code-generation investigation

The dual path is fixed without changing the network format, L1 arithmetic, quantisation, activation order, or scalar results. No GPU/trainer execution, downloads, pushes, cloud changes, or other jobs were involved.

## Cause: lost vectorisation in the final dual executable

Full executable disassembly is decisive. Library-only `cargo rustc --lib ... --emit asm` showed scalarised L2 code even for the default configuration, but that does **not** describe the final default executable after fat LTO.

- The actual `pawnstorm-xsearch5.exe` already has a compact L2 loop: broadcast one activation, four AVX `vmulps` + `vaddps` pairs, four YMM accumulator registers. It does not spill the L2 sums inside that loop.
- The old dual executable instead has scalar `vmulss` + `vaddss` for 32 separate outputs in **both** input passes. It repeatedly spills/reloads h2 on the stack. Adding the second loop caused LLVM to lose the default executable's vectorisation and scalarise the shared h2 values. The extra 512 products alone do not explain the slowdown; the first half of L2 also regressed.
- `evaluate` remains a call from evaluation/search. Pairwise packing and integer dot products are inlined in both builds. The new helper is inlined too; this does not add an inference call.
- Network alignment remains 64 bytes; the extra 16,384 bytes are a multiple of 64, L0/L1 offsets are unchanged, and every L2 bucket row is 128 bytes. L1Q construction/weights/scales/pointer use are unchanged. No layout repacking was needed.
- No subnormal post-L0 weights/biases exist in x25-big7 (checked all stored f32 values). The fix changes neither floating-point control registers nor rounding/denormal handling. No FMA instructions occur in the inspected before/after executable disassemblies. The code-generation/spill change explains the observed regression without a denormal or alignment workaround.

The SDK's already-installed dumpbin was used read-only, saving disassembly inside `dualact/target/l2-investigation/`. No SDK/tool downloads or installation. Reproduce assembly with the installed `dumpbin /DISASM <exe>` or `cargo rustc --bin pawnstorm --release -j 4 --offline --features dualact -- --emit asm`; inspect the **final executable/bin assembly**, not only the library output.

## Fix and exactness

On `dualact`, only the feature-on L2 path uses the new `layer2` helper. Form inputs `[clip(z)^2; clip(z)]`, then accumulate all 32 input rows in that exact order. AVX2 processes eight independent outputs per vector with four running sums; NEON processes four outputs per vector with eight running sums. Explicit separate multiply/add operations avoid FMA rounding. Scalar fallback performs the same ordered arithmetic. Default-off inference keeps its original loop.

Assembly excerpts from the actual executables are committed in `testing/l2-assembly.md`. The final and timed fixed-dual executables have identical 441,344-byte .text sections.

No zero-weight shortcut is used. The same kernel runs for a trained dual net with nonzero weights. Saved file format and trainer remain unchanged.

Checks:

- AVX2 dual: all seven library tests pass. 1,024 random L1 vectors spanning all eight buckets compare **all 32 raw L2 f32 bit patterns** with the scalar loop. This is in addition to exact end-to-end cp/reference and accumulator-refresh checks over **23,169 move positions**. Maximum difference from the unquantised trainer graph remains 3 cp on the synthetic test.
- Scalar x86-64 dual (`RUSTFLAGS=-C target-cpu=x86-64`, isolated `target/scalar`): same seven tests, bit checks and 23,169 positions pass.
- Default-off: all seven library tests pass.
- Fixed dual with a real x25-big7 net expanded by zero rows: bench depth 10 (all 12 bench FENs) **90,147 nodes**. Six separate depth-10 FENs: **13,189 / 16,184 / 16,228 / 3,248 / 11,164 / 6,121**, exact xsearch5 parity with every x25-big7 option applied.
- Every speed pair checks the entire per-position node vector, not just its total. Eight first bench FENs at depth 15: **1,167,068 nodes** per packet. This is an independent FEN selection from the referenced scratchpad's 876,344-node set; all FENs are recorded in JSON. The net transformation and test method are the same, recreated inside the allowed workspace.

## Paired timings

Below-normal priority, one engine/thread at a time, pinned P-core, execution-speed power throttling explicitly disabled. Each comparison has a warm-up pair then **six measured pairs**, alternating A/B and B/A. Net load/TT reset are outside the timed searches. Three-packet sets repeat the same eight searches three times with ucinewgame between every search (3,501,204 nodes per process). Wall timing measures go -> bestmove and is primary; extended sets also record engine-reported milliseconds.

| Comparison B vs xsearch5 | CPU | Packets/run | Median paired B/A NPS | Ratio min..max |
| --- | ---: | ---: | ---: | ---: |
| Old dual | 2 | 1 | 0.731901 | 0.719234..0.735205 |
| Fixed dual | 2 | 1 | **0.981897** | 0.953460..0.998323 |
| Old dual | 6 | 3 | 0.715925 | 0.688005..0.737241 |
| Fixed dual | 6 | 3 | **0.971462** | 0.932582..1.002982 |
| Fixed dual, confirmation | 6 | 3 | **0.981532** | 0.965860..0.991204 |
| Fixed dual, noisy extended set | 2 | 3 | 0.943443 | 0.757504..1.183847 |

Confirmation median absolute NPS: xsearch5 **1,802,093**, fixed dual **1,770,289**. Median paired ratio is the statistic of interest, rather than the ratio of those two medians. The main regression is removed: about **1.8% lower NPS** in the confirmation, near the requested 3% target. The earlier CPU6 set was 2.85% lower. Other CPU jobs remained running; a CPU2 extended set had severe contention/outliers and a 5.7% median loss, retained above rather than discarded. These are local speed measurements, not playing-strength or trained-net accuracy evidence.

Raw data for **all** sets, including unsuccessful default-path variants, is committed under `testing/l2-results/`. Compiles/tests were not run concurrently with the measured search sets.

## Default investigation / l2fast

The final default executable already vectorises L2, so the condition for a necessary default-path fix was **not** met. A separate `l2fast` branch was created from xsearch 94f4385 to test explicit SIMD anyway. Its first version triggered full unrolling and broadcast spills in library output; a compact-loop candidate hid only the activation slice metadata from LLVM on x86. This remained mathematically exact, but produced **no demonstrated speed benefit** over the already-vectorised shipped executable:

| Default SIMD candidate vs xsearch5 | CPU | Packets/run | Median paired ratio | Min..max |
| --- | ---: | ---: | ---: | ---: |
| Initial explicit SIMD | 2 | 1 | 0.989402 | 0.966529..1.176447 |
| Compact loop | 2 | 1 | 0.999618 | 0.943011..1.115683 |
| Compact loop | 6 | 3 | 0.989771 | 0.974747..1.018616 |

Candidate native tests passed (1,024 raw-bit L2 vectors, 23,274 move positions, all seven library tests). Bench + six-position parity also matched exactly, as did every timed depth-15 tree. There is no basis for promoting that change. **No runtime change is retained on l2fast**; it records this finding and preserves the exact tested candidate as `testing/l2fast-candidate.patch` for reproducibility. Applying the patch recreates the tested candidate. The built candidate remains locally at `dualact/target/l2-investigation/l2fast-candidate.exe`. Do not call this a default speed improvement.

## Reproduction (from lab root, PowerShell)

Check `testing/health/control.txt` first; wait if pause. Everything here uses CPU only and cached crates.

```powershell
(Get-Process -Id $PID).PriorityClass = 'BelowNormal'
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
Push-Location dualact
cargo build --release -j 4 --offline --features dualact
New-Item -ItemType Directory -Force target/l2-investigation | Out-Null
Copy-Item target/release/pawnstorm.exe target/l2-investigation/dual-fast-final.exe
cargo test --release -j 4 --offline --features dualact --lib -- --test-threads=1 --nocapture
Pop-Location
# Insert 16,384 zero bytes after the original 16-input L2 weights; retain its 64-byte padding.
py -3 -c "from pathlib import Path; d=Path('testing/engines/x25-big7.nnue').read_bytes(); n=2116096; Path('dualact/target/l2-investigation/big7-dualzero.nnue').write_bytes(d[:n]+bytes(16384)+d[n:])"
py -3 dualact/testing/l2_speedab.py --aexe testing/engines/pawnstorm-xsearch5.exe --anet testing/engines/x25-big7.nnue --bexe dualact/target/l2-investigation/dual-fast-final.exe --bnet dualact/target/l2-investigation/big7-dualzero.nnue --out dualact/target/l2-investigation/recheck.json --repeats 3 --cpu 6
```

To reproduce the default candidate: in clean `l2fast`, `git apply testing/l2fast-candidate.patch`, then `cargo build --release -j 4 --offline` and `cargo test --release -j 4 --offline --lib -- --test-threads=1 --nocapture`; compare its exe using the same runner with x25-big7.nnue for both A and B. The patch includes its stronger reference tests.

`dualact-cloud` is unchanged by this follow-up and still lacks this fix. Claude must cherry-pick the new dualact fix into that branch (or recreate its default-feature-only commit above current dualact) before cloud tests. No trainer change is needed. On ARM, Claude must run the raw-bit/reference tests, check generated NEON code uses separate FMUL/FADD rather than FMLA, compare identical-net x86/ARM direct evaluations and fixed-depth nodes, and repeat pinned timing. ARM execution/compilation was not available locally.
