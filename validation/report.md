# Frozen search parameters — local validation, 2026-10-30

Branch: `frozen`, based on v26cand `e6bfdf0`. Implementation commit:
`47e73f5` (`Add opt-in frozen search parameter builds and local validation`).
A following commit records this report, experiment notes and small harness/doc refinements.
No `-o` pathname was supplied, so this report is `validation/report.md`.
No push, deployment, remote operation, GPU work, downloads, installs, or changes
to other worktrees or existing jobs were performed.

## Design and files

`Cargo.toml` adds `frozen = []`, off by default. `src/param_defs.rs` contains the
unchanged parameter declaration list, shared by the engine macro and `build.rs`.
The build script uses only std, reads `frozen.options` when the feature is enabled,
starts from every DEFS default, and generates an i32 `const FROZEN_VALUES` array
in OUT_DIR. Names are case-insensitive. Blank lines and full-line `#` comments
are accepted; repeated names have last-assignment semantics. Unknown names,
malformed lines/integers and out-of-range values panic the build script and fail
the build. Cargo watches both the declarations and options file.

`src/params.rs`: the always-inline `p()` indexes that const under `frozen`.
Literal enum arguments can fold to literals, eliminating mutable parameter loads
and allowing disabled experiments and parameter arithmetic to fold. Frozen
`generation()` is always zero: Searcher::new already constructs the LMR table
from p(), so it contains the compiled settings from construction and never needs
rebuilding. With the feature off, VALUES, clamping, generation increments and
all search code paths remain as before.

`frozen.options` was copied directly from `../testing/engines/x25-big7.options`;
the working files were checked byte-for-byte equal after generator validation.
In particular it includes TmSoftPct=80, SprMatScale=400 and SprOptimism=100.
Parameters omitted from that file retain their declared defaults.

The existing UCI option listing is unchanged (including DEFS' advertised mutable
defaults). Frozen parameter assignments equal to the compiled value are silent;
different integers, including out-of-range integers, print exactly
`info string <Name> is frozen at <v> in this build` and do not change anything.
Other UCI options keep working normally. The guard is in params::set(), so all
parameter setters obey it. No search struct layouts or optional storage changes
were made, keeping this comparison confined to parameter freezing.

`src/search.rs` changes only its existing test module: three tests assumed
SprMatScale=0, whereas the required dev settings use 400. Their evaluation
expectations and alpha/beta windows now account for the compiled scales while
retaining the TT, optimism and correction-history assertions. An explicit
halfmove-fade assertion still checks 1000 -> 600 before material scaling.
`README.md` documents the feature. `validation/*.py` preserve the offline build,
generator, parity, UCI and paired timing procedure; large artifacts, executables,
reference archive, build directories, logs and raw JSON stay ignored locally.

## Correctness evidence

All runs use x25-big7.nnue, every x25-big7.options entry over UCI, Threads=1,
Hash=16. Each position starts with ucinewgame and readiness synchronization,
clearing TT/history. The reference is a git archive of e6bfdf0 inside this
worktree, built with the same compiler flags; the active v26cand worktree was
not built in or modified.

Every comparison checks **each position's nodes, score (cp/mate), bestmove,
ponder move and entire final PV**, rather than only summed nodes.

| ISA | Candidate vs v26cand | Depth 15: 12 positions, both totals | Depth 17: 8 positions, both totals |
|---|---|---:|---:|
| Native | feature off | 1,190,866, all pass | 2,885,408, all pass |
| Native | frozen | 1,190,866, all pass | 2,885,408, all pass |
| AVX2-only | feature off | 1,190,866, all pass | 2,885,408, all pass |
| AVX2-only | frozen | 1,190,866, all pass | 2,885,408, all pass |

This is 80 successful position comparisons. The timing rounds also enforce the
same full parity for every paired position. Per-position FENs, final info/PV,
nodes, scores and moves are in `parity-*.json` and `speed-*.json`. Sidecar
`*.meta.json` files save executable SHA-256s, options, net SHA-256, FENs, depths
and round counts; `base.json` records the reference commit.

Release cargo tests: **10/10** for reference, feature off and frozen, in both
native and AVX2-only builds (six passing runs). Tests run serialized with
RUST_TEST_THREADS=1. A parallel rerun exposed the existing NNUE-test global-net
race: incremental_matches_refresh calls set_net while TT tests read the net;
stored_eval_is_independent_of_halfmove_clock failed once. This is recorded in
`parallel-test-race.log`. It is unrelated to the feature and is not changed here;
serialized execution avoids it. Initial frozen tests also exposed the scale
assumptions described above, which were corrected before final validation.

Generator checks actually invoked `cargo build --release --offline -j 4
--features frozen` with unknown, high/low out-of-range, non-integer and malformed
input; all five builds failed as required. The original file was restored in a
finally block and the restored build succeeded. Separate fixture executions of
the compiled generator verify default fallback, whitespace, comments and
case-insensitive overrides. Both ISA builds pass frozen UCI tests: same-value
silence, exact canonical warning for different values and unchanged search
results after rejected assignments. See `generator-results.json`,
`generator-*.log`, and `uci-contract.json`.

## Speed

| Build | Paired wall NPS change, median [min, max] | Paired CPU NPS change, median [min, max] |
|---|---:|---:|
| Native | +5.20% [-1.64%, +7.82%] | +6.47% [+0.86%, +8.94%] |
| AVX2-only | +4.46% [+0.79%, +6.12%] | +6.64% [-4.10%, +11.08%] |

Every engine/round searched **9,393,267 nodes**, with full per-position parity.
Native wall ratios favored frozen in five of six rounds; native CPU ratios in
all six. AVX2 wall ratios favored frozen in all six; CPU ratios in five of six.

| Build / metric | Reference NPS median [min, max] | Frozen NPS median [min, max] |
|---|---:|---:|
| Native / wall | 1,203,861 [1,173,836, 1,354,587] | 1,280,176 [1,229,163, 1,384,985] |
| Native / CPU | 1,607,507 [1,495,445, 1,703,029] | 1,691,457 [1,603,118, 1,732,476] |
| AVX2-only / wall | 1,221,357 [1,171,408, 1,285,709] | 1,261,174 [1,241,866, 1,303,538] |
| AVX2-only / CPU | 1,592,571 [1,462,699, 1,607,404] | 1,635,862 [1,541,459, 1,722,548] |

The positive paired medians support an exact local speed gain for both builds.
The variation is too broad to assign a precise universal percentage; in
particular native has one negative wall pair and AVX2 one negative CPU pair.
Do not convert this short sample directly into an Elo claim.

Method: six paired rounds per ISA, all 12 bench FENs at depth 20. Two engine
processes maximum, only one searching at a time, position-by-position
interleaving with order reversed by round plus position. Both engines receive
an untimed depth-15 warm-up before the rounds. Each search clears history/TT;
net loading, clearing and startup are outside timing. No builds run during
timing. Windows engines run BelowNormal (verified), pinned to mask 1 (CPU 0,
P-core as in the earlier paired harness). SetProcessInformation explicitly opts
out of execution power throttling and API success is asserted. Wall time is
perf_counter from go to bestmove; CPU time is GetProcessTimes kernel+user over
the same interval. The controller reads testing/health/control.txt before each
launch/search/build, waiting on pause. No health pause was observed.
Builds use below-normal priority with --release --offline -j 4.

Native flags: `-C target-cpu=native`. AVX2-only flags:
`-C target-cpu=native -C target-feature=-avxvnni`. The latter means this PC's
native feature set with AVX-VNNI disabled, matching the prior comparison method;
it is not a portable deployment binary or an AMD measurement.

Percentages are medians of the six within-round frozen/reference NPS ratios.
Spreads are observed [minimum, maximum], not confidence intervals. CPU time
removes scheduling waits but not cache, memory or execution contention. The
training run and unrelated jobs remained untouched. These finite busy-PC
measurements establish neither Elo nor ARM/EPYC speed. Hardware retired
instruction counts were not measured.

## Reproduce locally

From the frozen worktree, with health control allowing work:

```powershell
py -3 validation/build.py
py -3 validation/generator.py
py -3 validation/verify.py
py -3 validation/speed.py
py -3 validation/summarize.py
```

The build script pins the archived reference to e6bfdf0, builds native and
AVX2-only reference/default/frozen executables into separate target directories,
and runs the existing tests with one test thread. The generator check temporarily
changes only this worktree's frozen.options and restores its exact bytes. Run
these scripts sequentially, with no builds during speed.py. Summaries are saved
in `summary.json`; all input/output artifacts remain under validation/.
The UCI harness reads this checkout's frozen.options for both engines, so future
winning-option updates also reach the mutable reference. This file currently
equals x25-big7.options. After changing it, rebuild and repeat parity before
timing; results in this report apply only to the committed dev settings.

Equivalent native frozen build (set the invoking shell's process priority to
BelowNormal first and check health control):

```powershell
$env:RUSTFLAGS = '-C target-cpu=native'
cargo build --release --offline -j 4 --features frozen --target-dir validation/target-native-frozen
$env:RUST_TEST_THREADS = '1'
cargo test --release --offline -j 4 --features frozen --target-dir validation/target-native-frozen
```

## ARM checklist for Claude — commands to run, not performed here

Use a separate, idle checkout of frozen, with the existing lab files at
`../testing/engines/x25-big7.{options,nnue}`. Do not replace the live bot or run
these against active deployment build directories. Use an available core within
the allowed CPU affinity set (BENCH_CPU below; change 0 if occupied/restricted).
No package installation or network access is required; if cached crates or the
existing net are missing, stop. Honor the local health control before every
command; the parity/pair harness also checks it before each search. The harness
supports Linux nice/affinity and process CPU time from /proc.

```bash
mkdir -p validation/arm-reference
git archive e6bfdf0 | tar -x -C validation/arm-reference
export RUSTFLAGS='-C target-cpu=neoverse-n1'
export RUST_TEST_THREADS=1
export BENCH_CPU=0
nice -n 10 cargo build --release --offline -j 4 --manifest-path validation/arm-reference/Cargo.toml --target-dir validation/target-arm-reference
nice -n 10 cargo build --release --offline -j 4 --target-dir validation/target-arm-default
nice -n 10 cargo build --release --offline -j 4 --features frozen --target-dir validation/target-arm-frozen
nice -n 10 cargo test --release --offline -j 4 --target-dir validation/target-arm-default
nice -n 10 cargo test --release --offline -j 4 --features frozen --target-dir validation/target-arm-frozen
```

Explicit neoverse-n1 enables the existing ARM dotprod path; do not substitute an
x86 flag. Before continuing verify those builds actually succeeded and the net
loads. Run both feature variants through both requested parity suites:

```bash
for candidate in default frozen; do
  nice -n 10 python3 validation/interleave.py validation/arm-$candidate-d15.json validation/target-arm-reference/release/pawnstorm validation/target-arm-$candidate/release/pawnstorm --depth 15 --count 12 --pairs 1
  nice -n 10 python3 validation/interleave.py validation/arm-$candidate-d17.json validation/target-arm-reference/release/pawnstorm validation/target-arm-$candidate/release/pawnstorm --depth 17 --count 8 --pairs 1
done
```

Then, without concurrent builds, run the interleaved bench pair:

```bash
nice -n 10 python3 validation/interleave.py validation/arm-speed.json validation/target-arm-reference/release/pawnstorm validation/target-arm-frozen/release/pawnstorm --depth 20 --count 12 --pairs 6
```

Each parity command asserts nodes/score/best/ponder/PV per FEN. Expected summed
nodes with this exact net/options are 1,190,866 (d15/12) and 2,885,408 (d17/8).
The bench pair enforces parity again; report round-summed nodes divided by
round-summed wall/CPU seconds, paired median ratios and min/max. Linux /proc CPU
time has tick resolution, so use whole-suite sums. ARM compilation, runtime
parity and speed remain unverified here. The new production code uses only std,
ordinary i32 constants and existing parameter machinery, with no x86 assumption
or new architecture-specific code.
