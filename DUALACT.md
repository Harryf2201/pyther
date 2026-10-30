# Dual activation handoff (2026-10-30)

Follow-up: [L2FAST.md](L2FAST.md) fixes the L2 code-generation regression. The speed figures below describe the initial implementation and are retained as historical evidence.

Experimental trainer + engine support; no GPU evaluation, training, games, remote operations or Elo claim. The default engine remains the xsearch architecture. The feature is build-time, not a UCI parameter. Expected accuracy/Elo benefit is unknown; measured speed cost is substantial, so validate before spending a full training budget.

## Branches and files

- `trainer-dualact`, based on `multiload` b42f9c5: `trainer/src/main.rs`, `tools/train.ps1`, this handoff.
- `dualact`, based on xsearch 94f4385: `Cargo.toml`, `src/nnue.rs`, `testing/dualact_verify.py`, `testing/dualact_agreement.py`, `testing/dualact-results.json`, this handoff.
- `dualact-cloud`: one commit above `dualact`, changing only Cargo's default feature list to `["dualact"]`. No workflow changes. Cloud runs must supply a trained mldual net; the old embedded/default net is incompatible.

## Architecture and binary contract

`mldual`: 768 inputs, shared 1024-wide feature transformer per perspective; clipped halves multiplied pairwise; concatenate side-to-move then other side (512 + 512). L1 is 1024 -> 16 per material bucket. For L1 preactivation z, concatenate `[clip(z,0,1)^2; clip(z,0,1)]` in that order. L2 is 32 -> 32 per bucket, then SCReLU, then 32 -> 1 per bucket; output scale 400 cp. Eight buckets: min(7, (piece_count - 2) / 4), kings included. The engine remains fixed at width 1024 / eight buckets; do not train other widths for this build.

No header. Little endian. All post-transformer weights are f32 and input-major, with all buckets' outputs adjacent for each input. File sequence (offsets in bytes):

| Field | Shape | Type | Offset | Bytes |
| --- | --- | --- | ---: | ---: |
| l0w | [768][1024] | i16, round(weight * 255) | 0 | 1572864 |
| l0b | [1024] | i16, round(bias * 255) | 1572864 | 2048 |
| l1w | [1024][8][16] | f32 | 1574912 | 524288 |
| l1b | [8][16] | f32 | 2099200 | 512 |
| l2w | [32][8][32] | f32 | 2099712 | 32768 |
| l2b | [8][32] | f32 | 2132480 | 1024 |
| l3w | [32][8] | f32 | 2133504 | 1024 |
| l3b | [8] | f32 | 2134528 | 32 |

Dual net: **2,134,560 bytes**, or **2,134,592** with bullet's final 64-byte padding. Original ml: 2,118,176 / 2,118,208 bytes. Increase: 16,384 bytes. Exact size validation accepts only unpadded or padded length, rejecting other architectures in either build. A UCI load of x25-big7 in the dual build reported `network file is 2118208 bytes, expected 2134560 (or 2134592 padded), dualact=true`.

Engine approximation is unchanged: clip L0 accumulators to 0..255; pair inputs round(a*b/512); quantise each f32 L1 output row to i8 with scale 127/max(abs(weight)), minimum max 1e-6. Convert dot products back with 512/(65025*scale), add f32 bias, then apply both activations. This is not bit-exact to the float trainer, including existing L0 saving error. L2 and L3 retain f32 weight precision. The small-layer additions keep input order, with all squared inputs before all linear inputs; no new FMA intrinsics or architecture-specific float code.

## Build and train (commands from lab root)

All CPU processes below normal priority; no more than four cargo jobs. Check `testing/health/control.txt` before commands, and wait if it says pause. No network is required. Trainer built offline successfully into **trainer-dualact/trainer/target** with CUDA 13.3; it was never executed. One existing linker warning remains: LNK4098 LIBCMT conflict.

```powershell
(Get-Process -Id $PID).PriorityClass = 'BelowNormal'
$env:CUDA_PATH = 'C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.3'
$env:Path = "$env:CUDA_PATH\bin\x64;$env:CUDA_PATH\bin;$env:USERPROFILE\.cargo\bin;$env:Path"
$env:CARGO_TARGET_DIR = "$PWD\trainer-dualact\trainer\target"
cargo build --manifest-path trainer-dualact/trainer/Cargo.toml --release -j 4 --offline
```

When the GPU is available, Claude can run this exact command from the lab root (not executed in this task):

```powershell
.\trainer-dualact\tools\train.ps1 -Name dualact-big7-run1 -Data .\testing\training\data\big7-list.txt -Superbatches 40 -Wdl 0.4 -Hidden 1024 -Buckets 8 -Inputs mldual -Build
```

The script now pins its own target directory and builds offline with four jobs. It starts training with below-normal priority and four loader threads. Choose a fresh Name for reruns. An ml checkpoint has incompatible L2 dimensions; train mldual from scratch unless explicitly implementing a checkpoint conversion later.

Build/test engine from `dualact` (unset CARGO_TARGET_DIR after the trainer build):

```powershell
Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
Set-Location dualact
cargo build --release -j 4 --offline
New-Item -ItemType Directory -Force target/dualact-check | Out-Null
Copy-Item target/release/pawnstorm.exe target/dualact-check/default.exe
cargo build --release -j 4 --offline --features dualact
Copy-Item target/release/pawnstorm.exe target/dualact-check/dualact.exe
cargo test --release -j 4 --offline --features dualact --lib -- --test-threads=1 --nocapture
cargo test --release -j 4 --offline --lib -- --test-threads=1 --nocapture
py -3 testing/dualact_verify.py
```

## Evidence

Both native default and dual builds passed all seven library tests. The network test now uses finite random f32 small-layer weights and i16 L0 values, validates file size errors, refreshes accumulators after random legal moves, and compares incremental inference exactly to an independent scalar forward pass modelling deployed rounding. It additionally evaluates the unquantised trainer formulas (using saved L0 values divided by 255); tolerance in this bounded synthetic test is 8 cp, not a universal trained-net error guarantee.

- AVX2 dual: 23,169 positions, exact deployed-reference agreement; maximum float graph difference 3 cp.
- AVX2 default: 23,274 positions, exact deployed-reference agreement; maximum graph difference 2 cp.
- Scalar x86-64 dual (`RUSTFLAGS='-C target-cpu=x86-64'`, `CARGO_TARGET_DIR=target/scalar`): all seven tests passed; same 23,169 positions / 3 cp as AVX2.

Default parity against pawnstorm-xsearch5.exe with x25-big7.nnue and **every** x25-big7.options entry: bench depth 10 (all 12 positions) **90,147 nodes**, exact match. Six independent depth-10 searches, clearing search/TT each time (first six bench positions in source order): **13,189; 16,184; 16,228; 3,248; 11,164; 6,121**, all exact matches.

Speed script uses below-normal engines, one thread, P-core logical CPU 2 (affinity mask 4), and successfully disables execution-speed power throttling using SetProcessInformation. Eight alternating paired bench-depth-11 runs; first pair discarded as warm-up. Synthetic nets share identical finite random weights; added CReLU weights are zero, preserving identical evaluations/search trees while executing the new runtime layout. Each run: **3,537,971 nodes**. Other lab CPU jobs remained running.

| Seven measured runs | Min NPS | Median NPS | Max NPS |
| --- | ---: | ---: | ---: |
| Default | 1489045 | 1546315 | 1578042 |
| Dual | 1097726 | 1124593 | 1155067 |

Median paired dual/default ratio **0.7422**, i.e. **25.8% lower NPS** (34.7% more time at equal nodes). Individual ratios 0.6956..0.7634. This is a synthetic-network local estimate under contention, not an almost-free change or an Elo prediction. A trained network changes the tree and may change the measured cost; remeasure on ARM and cloud before drawing playing-strength conclusions. Raw numbers are committed in `testing/dualact-results.json`; detailed local logs and synthetic nets are under ignored `target/dualact-check/`.

## Required trainer/engine check after training

Prepare a FEN file with one complete FEN per line: both sides to move, all eight material buckets, promotions, en passant, castling, sparse endings and middlegames; preferably thousands of positions from our own games. Do not use other engines as teachers. Use the **same checkpoint's** optimiser/model state for trainer evaluation and its saved quantised network for the engine. The checkpoint directory is the actual epoch checkpoint, not its parent; locate its quantised file rather than guessing a filename.

GPU work below is for Claude **after** the existing run finishes. From lab root, with CUDA environment set as in the build command:

```powershell
$checkpoint = (Resolve-Path '.\testing\training\dualact-big7-run1\checkpoints\dualact-big7-run1-40').Path
$data = (Resolve-Path '.\testing\training\data\big7-list.txt').Path
$fens = (Resolve-Path '.\dualact\target\dualact-check\agreement.fens').Path
& .\trainer-dualact\trainer\target\release\trainer.exe dualact-big7-run1 $data $checkpoint 40 0.4 1024 8 mldual "eval=$fens" > .\dualact\target\dualact-check\trainer-eval.tsv
$net = Join-Path $checkpoint 'quantised.bin'
py -3 .\dualact\testing\dualact_agreement.py .\dualact\target\dualact-check\trainer-eval.tsv $net
```

Checkpoint suffix/path above must match the actual checkpoint directory produced by the run. `eval=` must be argument 9, immediately after mldual; it loads the checkpoint and returns without training, but still constructs CUDA graph state and reads the supplied training data list. Trainer TSV now prints eight decimal places. Its values are **raw network outputs, not cp**; the comparison helper multiplies by 400 and compares the engine's direct `nnue:` evaluation, not search scores or material-scaled UCI scores. It reports signed bias, MAE, p95, maximum and RMS cp error, plus each FEN. PowerShell 5.1 redirection writes UTF-16, which the comparison helper accepts.

Inspect outliers by side/material bucket and activation saturation. Some error is expected from L0 i16 saving, pairwise rounding, L1 i8 quantisation and final integer truncation. The synthetic 3 cp observation is not a trained-model acceptance bound. A swapped concat half, bucket, transposition, scale or L2 stride can cause large systematic differences; resolve those before games. Record trained-model errors and compare to the existing ml pipeline's quantisation baseline. No checkpoint agreement has been measured here because running the trainer was prohibited.

## ARM / cloud handoff

Claude must build/test dualact on aarch64 NEON, run the same random-network test and verify the same 23,169 positions / 3 cp diagnostic, then compare direct `nnue:` values against x86 for the trained net and the same FENs. Verify default-off x25-big7 node parity on ARM, and compare dual fixed-depth node counts across ARM/x86 with identical net/options. Existing integer NEON code is unchanged; new activation/L2 additions are shared Rust, with the original operation order preserved. ARM compilation/execution was not available here. Repeat pinned bench timing on ARM; local synthetic speed loss may outweigh accuracy. Claude should dispatch any cloud games using dualact-cloud plus an mldual net, then check strength at both brief time controls. No pushes, cloud dispatches or release changes were made.
