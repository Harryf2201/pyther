# armspeed: exact ARM candidates and measurement kit

Local branch `armspeed`, based on `v26rc` / `4f58f61`. Three retained candidate
commits follow profiling-kit commit `4ec7bf4`. Nothing was pushed or deployed.
ARM execution and N1 performance remain for Claude to establish on the server.

| Commit | Change | Expected ARM effect |
|---|---|---|
| `15e9bc1` | Pack runtime L2/L3 weights by output bucket | Contiguous L2 streams; L3's selected weights occupy two cache lines rather than sixteen |
| `768fa45` | Process 16 pairwise activations per iteration/store | Half as many loop iterations and output stores; two independent multiply chains |
| `893844f` | Four-output dense SDOT kernel, two accumulators/output | Share input loads across four outputs and avoid activation-vector register spills |

Final measurement-kit repair `c900ff0` fills the activation buffer in place.
Extraction initially introduced a 1 KB per-evaluation return-value copy that
showed up in final ARM assembly. This repair removes it; it is not attributed as
a speed-up over v26rc, which had no such copy. The final source to benchmark is
`c900ff0` (three candidates plus the repaired kit).

Commits are cumulative. Measure each against its predecessor and the final tree
against `4f58f61`. Measure the kit itself against `4f58f61`, too: extracting
benchmarkable kernels changes compiler inlining/outlining, even with counters off.
No x86 sparse/dense dispatch policy, network format, search parameter, TT policy,
or floating-point arithmetic order was changed.

## Measurement kit

```
pawnstorm profile [iterations=20000] [depth=12] [EvalFile] [options-file]
```

The command runs single-threaded with Hash=16 and fresh TT/history for each of
the 12 existing bench FENs. The micro corpus includes those boards, every legal
move from them and its resulting board, plus an en-passant fixture: 332 boards,
290 quiet / 23 capture / 3 castle / 4 promotion update fixtures. Promotion captures
are classified as promotions; en passant as capture. Each operation is warmed,
then timed in five amortized batches; the output gives median and min/max ns/op.
`black_box` protects inputs and outputs. The final tool also prints a selection
overhead calibration. Reported costs include harness selection/black-box overhead.

It times full refresh, each update category, one side's pairwise activation,
all 16 dense L1 dots, all 16 sparse L1 dots, L2/L3 including L1 dequantization and
bias, complete evaluate(), Board copy plus make_move, pseudo/legal movegen, SEE
at threshold zero, and TT probe/store. Legal movegen includes allocation/filtering;
TT micro access is warm and mostly hits. Both integer L1 kernels are compared on
the corpus before timing. A digest combines evaluation integers and raw final
float bits; compare it across candidates built for the same platform/net.

`--features profile-counters` adds thread-local non-atomic counters, enabled only
during this command's search phase. Normal builds compile every counter hook and
its argument evaluation out. Instrumented binaries are for call counts, not A/B
speed. Each share is `calls * median_ns / instrumented_search_ns`; evaluate
contains pairwise/L1/tail, and legal movegen contains pseudo generation. Do not
sum nested rows, or expect independently timed child kernels to sum to their
inlined parent's time. Cache state, branch distributions, timer floor, counter
overhead and inlining make these estimates approximate. Counts are the firmer
evidence; this is not a replacement for a sampling profiler.

Changed files: `Cargo.toml`, `src/{profile,lib,main,bench,nnue,board,tt}.rs`;
`validation/armspeed.py`, `armspeed-asm.py`, `armspeed-summary.py`,
`armspeed-arm.sh`, this report and experiment/documentation notes. `board.rs`
and `tt.rs` only receive feature-gated counter hooks. `search.rs` and `attacks.rs`
were reviewed and left unchanged.

## x86 reference profile

Native AVX2+AVX-VNNI, x25-big7 net and all its options, counters enabled only in
the fixed-depth search. 100,000 iterations per batch, five batches, depth 15.
Process affinity CPU 0, BelowNormal; other existing PC jobs were left running.
These are a reference sample, not N1 predictions. They describe the original
kit; the final in-place-buffer profiles are retained separately as
`as-inplace-*-profile.log`.

| Component | Median ns/op | Calls at d15 | Estimated search share |
|---|---:|---:|---:|
| Refresh | 1387.27 | 12 | <0.01% |
| Update quiet | 103.38 | 722358 | 7.80% |
| Update capture | 119.55 | 342630 | 4.28% |
| Update castle | 184.05 | 12813 | 0.25% |
| Update promotion | 105.88 | 5637 | 0.06% |
| Pairwise, one side | 35.88 | 1815630 | 6.81% |
| Dense L1 | 165.33 | 907815 | 15.69% |
| Sparse L1 | 102.65 | 0 | 0% (not selected on this native build) |
| L2/L3 incl. dequantization | 43.86 | 907815 | 4.16% |
| Whole evaluate | 222.07 | 907815 | 21.07% |
| Copy + make_move | 16.71 | 1083439 | 1.89% |
| Pseudo movegen | 60.81 | 413112 | 2.63% |
| Legal movegen | 252.69 | 12 | <0.01% |
| SEE(0) | 7.44 | 1963976 | 1.53% |
| TT probe | 2.49 | 1184864 | 0.31% |
| TT store | 4.35 | 387268 | 0.18% |

The counted search visited **1,190,866 nodes in 0.956849 seconds**, 1,244,571 NPS.
Raw sample: `validation/as-kit-counters-profile.log`. Independent tail sample
measured 40.59 ns versus the reference's 43.86 ns, but scheduling/frequency noise
also moved unaffected components. Use the paired search measurements below for
the x86 speed observation.

## Exactness and x86 speed

All comparisons use Threads=1, Hash=16, no Syzygy, fresh TT/history per FEN,
`testing/engines/x25-big7.nnue` and every x25-big7 option. The harness verifies
that the actual options file matches `frozen.options`. Every requested
per-position comparison asserts nodes, score, best move, ponder and full PV.

Each of the three retained candidate commits passes native normal and frozen parity
against the feature-matched v26rc reference. The final candidate also passes
AVX2-only and generic x86-64 (no AVX2/BMI2) normal/frozen builds and parity.
Counter-feature normal/frozen trees are compared separately as well.

* Configured depth 15, all 12 FENs: **1,190,866 nodes** for every variant.
* Default CLI `bench` (embedded net, depth 10): **146,622 normal**, **177,339
  frozen**. Different compiled default options explain the difference; compare
  bench only to a feature-matched reference.
* Configured depth 20, all 12 FENs: **9,393,267 nodes** in every timing round.
* Corpus digest: **`7f34c575ad382200`**, including final-float bits, matching
  native, AVX2-only and generic-kernel runs.
* Release tests: **17 passed / 1 ignored** normal; **14 passed** frozen, serial
  because the existing suite changes a process-global network. The ignored
  normal test needs a real Syzygy table and is unrelated to these changes.
* Tests cover integer dot/sparse scalar oracles, pairwise rounding, >10,000
  incremental-vs-refresh moves, and an original-layout floating-point oracle
  comparing raw output bits over 512 synthetic cases/all buckets. ARM-only
  tests additionally cover shared-input dots;
  those were cross-checked for compilation, not executed here.

Native paired depth-20 timings use position-interleaved A/B order, reversing
order by round and FEN, a depth-15 warm-up, and aggregate nodes / aggregate wall
or process CPU seconds. No builds ran during these timings. Ranges below are
observed round spreads, not confidence intervals.

| Comparison | Pairs | Reference / candidate median NPS | Paired median wall change [min,max] | Paired median CPU change |
|---|---:|---:|---:|---:|
| Tail vs kit | 6 | 1,257,777 / 1,305,784 | +3.31% [-0.19%, +6.21%] | +2.43% |
| Pairwise vs tail | 3 | 1,466,824 / 1,456,898 | +0.18% [-2.30%, +2.24%] | -1.32% |
| Shared dot vs pairwise | 3 | 1,402,420 / 1,413,918 | +0.17% [+0.10%, +1.55%] | +0.29% |
| Three candidates before kit copy repair vs v26rc | 6 | 1,195,273 / 1,173,028 | -1.04% [-5.79%, -0.31%] | -0.12% |
| Kit copy repair vs shared dot | 3 | 1,376,711 / 1,423,933 | +3.47% [+2.83%, +3.98%] | +0.59% |
| Final repaired kit + candidates vs v26rc | 6 | 1,262,911 / 1,278,559 | -0.55% [-3.55%, +5.13%] | +1.05% |

The last two retained candidate commits change only AArch64 code; their x86 timing
variation is a control/noise observation, not an expected speed-up. No ARM or Elo
gain is claimed. The final x86 comparison shows **no resolved overall gain**;
the positive tail result must not be presented as the total engine gain.
Raw parity, build/test logs, hashes, profiles, timing rounds and
summaries stay local under ignored `validation/as-*`. The report records the
numbers needed to assess the public source without committing those artifacts.

## AArch64 assembly review

Installed `aarch64-unknown-linux-gnu` target; rustc **1.99.0
(b940084d7 2026-09-28)**. Offline assembly uses
`-C target-cpu=neoverse-n1`. `rustc --print cfg` confirms NEON and dotprod.
Both intermediate library assembly and **final binary assembly after fat LTO**
were inspected. Binary assembly is emitted before the expected `linker cc not
found` error on Windows; no ARM executable is linked or run. The helper accepts
only that specific missing-linker failure and requires assembly to exist.
Other compilation failures remain errors. Also emitted the `-C
target-feature=-dotprod` widening/sparse library fallback and frozen+counter
build; `cargo check --tests` covers ARM-only test bodies. No installation,
download or profiler was used.

**Baseline final fat-LTO accumulator updates already vectorize.** They use
paired Q-register loads/stores and 36 static NEON i16 add/sub instructions across
the branches, with no scalar halfword loads. Intermediate library output showed
scalar loops; that is not evidence about the shipping executable. An
explicit-NEON trial (`7047bc7`) passed x86 parity but was reverted by `6496d2c`
after this distinction was checked. It is excluded from retained candidates and
the ARM command list. Refresh was inlined in final binary output and is not a
standalone summary entry.

The original **final-LTO** evaluate has 84 static stack-vector load instructions
and 28 stores while hoisting/unrolling dense L1 activations. Final repaired
evaluate has **0 such vector loads / 4 stores** (433 static instructions);
the separate SDOT kernel has **zero stack traffic**, 169 instructions and 32
static SDOTs across four bounded loops, with eight running vectors. The 1 KB
helper-return `memcpy` is absent after `c900ff0`; the original zero initialization
remains. Counts describe emitted instructions,
not dynamic load totals or measured speed ratios. The complete evaluation still
has stack traffic for other stages. The kit's `array::from_fn` helper can be
outlined differently from the original, another reason to measure the kit itself
and the total candidate.

Pairwise assembly emits `sqrdmulh`, `sqxtun`/`sqxtun2` and a `str q` for 16
outputs, replacing the original eight-output `str d`. Bounds are multiples of
the vector width and no partial vector overreads occur. Dot inputs remain
0..127 and weights -128..127; the complete worst-case magnitude is 16,646,144,
well inside i32, so integer reassociation is exact.

Accumulator rows already have 64-byte alignment and fused copy/update passes.
The candidate retains them. New tail rows are aligned to 64 bytes and add
**17,408 weight bytes per loaded net** plus object alignment padding, shared
across threads. Serialized
weights, quantization, biases and bucket arithmetic stay unchanged. Bucket
selection lowers to saturation/shift/min; no new per-node material scans.

L2 operates across independent output lanes and L3 preserves its scalar
left-to-right sum. Assembly contains no FMA in these kernels; introducing fused
multiply-add or a reordered horizontal floating sum could change bits and was
not used. Integer horizontal sums are safe. Existing ctz/popcount intrinsics
and non-BMI2 magic bitboards remain; ctz lowers to `rbit`/`clz`. Existing ARM TT/null-child prefetching,
node-local history keys and qsearch SEE reuse already address those ideas.
Extra weight prefetches, a changed Board layout and another lazy-accumulator
experiment were not added without evidence. The prior lazy trial regressed.

Inspect the same summaries locally:

```powershell
py -3 validation/armspeed.py build repro-arm --ref c900ff0 --arch arm --final-asm --test
py -3 validation/armspeed.py build repro-neon --ref c900ff0 --arch arm-neon --asm --test
py -3 validation/armspeed-asm.py repro-arm repro-neon
```

These invoke `cargo rustc --bin pawnstorm` (final fat LTO) or `--lib`
(intermediate) with `--release --offline -j 4 --target aarch64-unknown-linux-gnu
-- --emit=asm`, plus `cargo check --tests`, in separate target directories.
Final-assembly mode records the expected unavailable ARM linker separately.

## Local reproduction

Run sequentially from the `armspeed` worktree. Existing cached crates and lab
net/options are required; no package installation is allowed. Choose fresh
labels if reproducing again: archived reference snapshots are never overwritten.

```powershell
py -3 validation/armspeed.py build repro-base --ref 4f58f61 --test
py -3 validation/armspeed.py build repro-final --ref c900ff0 --test
py -3 validation/armspeed.py check repro-final repro-base
py -3 validation/armspeed.py build repro-base-frozen --ref 4f58f61 --features frozen --test
py -3 validation/armspeed.py build repro-final-frozen --ref c900ff0 --features frozen --test
py -3 validation/armspeed.py check repro-final-frozen repro-base-frozen
py -3 validation/armspeed.py build repro-counted --ref c900ff0 --features frozen,profile-counters --test
py -3 validation/armspeed.py profile repro-counted --iterations 100000 --depth 15
py -3 validation/armspeed.py speed repro-final repro-base --pairs 6 --depth 20
py -3 validation/armspeed-summary.py validation/as-repro-final-speed.json
```

Use `--arch avx2` or `--arch scalar` for the other x86 paths. Build/check each
candidate ref from the table the same way; compare frozen to a frozen baseline.
Cargo always uses `--release --offline -j 4`; tests are serialized. Windows
children are BelowNormal and engines pinned to CPU 0; set `BENCH_CPU` to change
the core. The watchdog suspends only this kit's child processes on health pause
(cargo descendants included) and resumes when the control permits. At most two
engine processes are launched. Nothing starts or stops unrelated lab jobs.

## Exact ARM command list for Claude

After Claude makes these commits available remotely, use a separate checkout of
`armspeed` as the handoff directory. The script does not fetch or push; only the
existing `arm-ab.sh` mode fetches origin as specified by the owner. No bot binary,
service, deployment worktree, Oracle setting or GitHub workflow is changed.

Set these paths to the existing source clone/lab and a **new** experiment directory.
Select an available core allowed by server affinity. Do not run these stages in
parallel, or while health control requests pause. The helper uses nice +10,
offline cargo/-j4, one allowed core for builds and searches, a health watchdog,
separate worktrees for all six benchmark refs and separate target dirs for every feature set.

```bash
# In the new armspeed handoff checkout (not the live deployment checkout):
export REPO="$HOME/pawnstorm"             # existing git clone; adjust if needed
export PAWNSTORM_LAB="$HOME/testing"      # existing x25-big7 net/options; adjust
export ARM_LAB="$HOME/armspeed-lab-20261030" # must not exist yet
export BENCH_CPU=0                       # choose an available allowed core
bash validation/armspeed-arm.sh build
bash validation/armspeed-arm.sh profile
bash validation/armspeed-arm.sh speed
python3 validation/armspeed-summary.py validation/as-arm-*-speed.json
```

`build` checks baseline/kit/all three candidates/final kit repair with and without
`--features frozen`, then builds five separate frozen+counter binaries and checks them.
`profile` runs each candidate/kit in normal, frozen and counted configurations.
Require `dotprod=true`, matching per-position nodes and matching digest
`7f34c575ad382200` for this exact net; the scalar floating oracle also runs on
the server, so do not rely on a cross-platform digest alone. `speed` performs six
alternating d20 pairs per predecessor and six final-vs-v26rc pairs for each
feature set, excluding counted builds. Every round asserts per-position parity.

Optional widening-NEON fallback execution on N1 (dotprod stays disabled for
these builds even though the hardware supports it):

```bash
python3 validation/armspeed.py build arm-final-neon --source "$ARM_LAB/final" --arch arm-neon --test
python3 validation/armspeed.py check arm-final-neon arm-base-normal
python3 validation/armspeed.py build arm-final-neon-frozen --source "$ARM_LAB/final" --arch arm-neon --features frozen --test
python3 validation/armspeed.py check arm-final-neon-frozen arm-base-frozen
```

The exact direct command shape produced for each source/features pair is:

```bash
RUSTFLAGS='-C target-cpu=native' nice -n 10 cargo build --release --offline -j 4 \
  --manifest-path "$ARM_LAB/final/Cargo.toml" \
  --target-dir validation/as-target-arm-final-frozen --features frozen
# Repeat without --features frozen into ...-normal, and for base/kit/tail/pair/dot.
# Use the script rather than this bare example for continuous health monitoring.
```

The supplied existing A/B helper can also run every comparison with the exact
interface the owner specified. Run sequentially after publishing the branch:

```bash
bash validation/armspeed-arm.sh ab
# Representative expanded calls (the script wraps them in the health watchdog):
CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=4 nice -n 10 "$HOME/arm-ab.sh" \
  as-base-normal 4f58f61 "" as-final-normal c900ff0 "" 6 20
CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=4 nice -n 10 "$HOME/arm-ab.sh" \
  as-base-frozen 4f58f61 "frozen" as-final-frozen c900ff0 "frozen" 6 20
```

The script includes base->kit->tail->pair->dot->final and final-vs-base for both
feature sets. Existing helper logs follow `~/arm-ab-<nameA>-<nameB>.log`;
watchdog-wrapped stdout also stays in `validation/as-armab-*-command.log`.
The supplied `speed` mode explicitly enforces Threads=1 and at most two engines;
the existing arm-ab helper is an additional measurement, not a substitute for
those parity checks. No Threads=15 benchmark is prescribed under the six-thread
work limit. Scaling/cache effects at the bot's actual Threads=15 remain unknown.

## Risks and remaining uncertainty

All ARM runtime tests, node parity, counters and speed measurements are pending.
The assembly is from this installed compiler; the server's compiler may make
different inlining/vectorization decisions. Four-row weight streams may stress
N1 caches differently, and a non-inlined dense kernel adds a call. Packed tails
add shared cache footprint. Single-thread exact parity does not make SMP node
counts deterministic. Busy-PC x86 spreads and warm microbench estimates do not
establish an ARM speed gain, Threads=15 throughput or Elo. Keep only candidates
that pass the server's tests/parity and repeatably improve its measurements.
Discard and rerun timing samples spanning a health pause: elapsed wall clocks
include the suspended interval.
