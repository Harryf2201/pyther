# v26rc correctness candidate — 2026-10-30

Local branch `v26rc`, created from frozen `c73a3fe32e1ec399edde79c156ec64e965c0c00f`.
Implementation commit: `fffd930` (`Add default-off mate50, TB ply and rule50 TT fixes`).
This report and experiment notes are committed separately. No `-o` pathname was
supplied, so the tracked report is `validation/rc-report.md`; a final copy with
both commit IDs is saved in `../testing/coord/v26rc/2026-10-30/REPORT.md`.

## Changes

* `src/param_defs.rs`: independent UCI spin options `FixMate50`, `FixTbPly`,
  `FixR50Tt`, all default 0, range 0..1, using the existing parameter machinery.
* `src/search.rs`: mate precedence at the halfmove boundary in main search and
  qsearch; TB score ply conversion, including direct WDL TT stores; current-clock
  guard on every searched-score TT read, including eval estimates, ProbCut and
  singular extension. TT keys and replacement policy are unchanged.
* `src/rc_tests.rs`: numerical boundary tests, actual main/qsearch TT read tests,
  mate-boundary tests, independent-option eval clamps, and an opt-in real-table
  reproduction using the existing local net/options/tables.
* `validation/rc.py`: offline builds, reference archive, requested parity suites,
  UCI reproductions, and paired Threads 1/4 timing. Large evidence stays ignored.

`frozen.options` is unchanged. Its worktree SHA256 is
`445ed7f29fec3c4b370bea0cebb8184416806d45240246562e650d4a8977936f`.
The three new options compile to zero in the frozen feature build; attempts to
enable them through UCI correctly warn and retain zero. All-on integration and
timing therefore use the normal build. No version bump or strength claim.

## Score semantics and boundaries

`MAX_PLY=128`, `MATE=31000`, `MATE_BOUND=30872`, `TB_WIN=30743`,
`TB_BOUND=TB_WIN-MAX_PLY=30615`. The TB band is explicitly
`30615 <= |score| < 30872`. With either TT fix enabled, raw, adjusted and
corrected evals clamp to ±30614; both fixes off retain the old ±30871 clamps.
Scores below the reserved band pass through the TT conversion/guard unchanged.

FixTbPly adds/subtracts the current ply on write and reverses this on read,
exactly as for mate scores. Search still returns `±(TB_WIN - ply)` for a direct
WDL result, preserving preference for reaching the TB result sooner above the
local piece limit. This is not the rejected constant-search-score patch.
A real direct TB result normalizes to ±30743 in TT; generated TB results remain
below the mate band. Tests include the lower inclusive boundary, the upper
exclusive boundary, both signs, different store/read plies, and maximum horizon.
Synthetic upper-gap inputs round-trip, but do not represent direct WDL results;
normalization of an arbitrary score immediately below MATE_BOUND can cross that
threshold. Actual WDL-derived scores normalize to at most TB_WIN and avoid this
ambiguity. This assumes future heuristics do not invent forced scores in the gap.

FixR50Tt compares node-relative distance against `max(0,100-halfmove)` before
returning a decoded score. Distance **equal to or less than** remaining plies is
retained; a greater distance returns the same sign times 30614, outside both
forced-result bands. Mate distance is `MATE-|stored|`; normalized TB distance is
`TB_WIN-|stored|`. With FixTbPly off, the legacy TB score is root-relative, so the
guard subtracts the current ply to interpret it; this does not repair cross-ply
errors in legacy entries. Each option can be enabled independently.

## Validation

All substantive CPU work ran BelowNormal, cargo `--release --offline -j 4`,
serialized Rust tests, no more than two engine processes, one engine searching
at a time during paired timings, Threads 1 or 4. Health remained `auto`.
The harness monitors health during commands and searches; it stops only its own
build tree or suspends its own engine on pause. No GPU, downloads, installs,
remote operations, deployments, or changes to other worktrees/jobs.

Reference builds come from a git archive of c73a3fe inside this worktree.
Reference and candidate use **separate Cargo target directories**. An initial
shared-target attempt reused reference artifacts; its results were rejected and
all four builds/tests and all parity results below were obtained after separating
the targets. Native flags: `-C target-cpu=native`.

Every parity search uses x25-big7.nnue and every x25-big7.options setting,
Threads=1, Hash=16, fresh TT/history per FEN. Both option files were compared and
match. Net SHA256:
`3737bc586abde795f11378df935e6aa4cc1b1f429cff4c13bda45bca339f76de`.

| Candidate / c73a3fe reference | 12 positions, depth 15 | 8 positions, depth 17 |
|---|---:|---:|
| Normal / normal | all pass; both 1,190,866 nodes | all pass; both 2,885,408 nodes |
| Frozen / frozen | all pass; both 1,190,866 nodes | all pass; both 2,885,408 nodes |

All **40 per-position** comparisons match nodes, cp/mate score, bestmove, ponder
move and complete final PV. Evidence: `rc-parity-*.json` plus metadata sidecars.
The new options advertise default zero in both builds.

Release tests: reference normal/frozen **10/10** each; candidate normal
**15 passed, 1 optional local-table test ignored**; candidate frozen **12/12**.
The optional local-table test was then explicitly run and **passed**. Tests are
serial because the existing tests replace a process-global NNUE network.
The enabled numerical helper tests also run under frozen; mutable-UCI integration
tests run under normal. See `rc-*-test.log` and `rc-tb-reproduction.log` (last
successful runs; initial attempt diagnostics remain in some build/test logs).

## Findings 7 and 8, all on

At **both Threads 1 and 4**, without Syzygy, depth 3 on
`7k/8/5KQ1/8/8/8/8/8 w - - 99 1`:
all off returns cp 0 / `f6f5`; all on returns **mate 1 / `g6g7`**.
At clock 98, both configurations return mate 1 / g6g7. Unit tests check the
resulting mate at clocks 99 and 100 in main/qsearch, with contempt zero/nonzero,
and FixMate50 independent of both TT options.

Real WDL reproduction on `7k/8/8/8/8/8/8/1Q5K w - - 0 1`:

| Configuration | Search at ply 3 | Raw TT | Warm read at ply 7 | Cold probe at ply 7 |
|---|---:|---:|---:|---:|
| All off | 30740 | 30740 | 30740 | 30736 |
| All on | 30740 | 30743 | **30736** | **30736** |

The negative-score counterpart passes too. Actual negamax reads against a shared
TT pass with **1 and 4 simultaneous search workers**, for both signs and both
configurations. UCI Threads 1/4 also search a six-piece root outside the local
five-piece coverage, `7k/8/5K2/8/8/8/pr6/RR6 w - - 0 1`, depth 6:
both return **cp 30742 / b1b2**, reaching a TB result below the root. Raw evidence:
`rc-tb-reproduction.log`, `rc-reproduction-uci.json`, `rc-reproduction-uci.log`.

## Paired NPS

Six paired rounds per thread count; 12 FENs, depth 20; identical executable with
all three toggles off/on. Fresh TT/history, untimed d15 warm-up, order reversed
by position and round, no builds during timing. CPU affinity masks 1 (Threads 1)
and 15 (Threads 4); BelowNormal verified; per-process throttling opt-out verified.
Wall time runs from go to bestmove; CPU time is process kernel+user over the same
interval. Aggregate NPS = summed nodes / summed seconds for each round.

| Threads | Off median wall NPS | On median wall NPS | Paired wall change, median [min,max] | Paired CPU change, median [min,max] |
|---|---:|---:|---:|---:|
| 1 | 1,227,696 | 1,230,679 | **-0.72% [-3.89%,+2.99%]** | -0.86% [-4.43%,+2.96%] |
| 4 | 4,101,743 | 4,170,557 | **+2.04% [-3.27%,+6.94%]** | +1.99% [-1.23%,+5.42%] |

Threads 1 has identical per-position nodes/scores/moves/PVs throughout, exactly
9,393,267 nodes per configuration per round. SMP trees differ nondeterministically;
Threads 4 uses aggregate reported nodes, not an assertion of equivalent work.
This busy-PC sample shows **no resolved NPS regression**, but cannot prove zero
cost or rule out a small regression below its noise. Ranges are observed spreads,
not confidence intervals. No ARM, EPYC, Elo, or tablebase-heavy speed claim.
Evidence: `rc-speed*.json`, metadata and `rc-speed*-summary.json`.

## Known limits, especially DTZ

The cheap guard is not a general solution to deferred findings 1/2. WDL distance
is distance to a TB result, **not** distance to mate or to the next zeroing move.
Explicit reproduction with all three on, black to move in
`7k/8/5K1Q/8/8/8/8/8 b - - 0 1`: after caching the WDL result at ply 1,
changing only the clock to 99 still gives warm **-30742**, versus cold **0**.
The guard sees a zero-distance WDL result and cannot reject it from this score
alone. Clock/history keys remain unchanged as requested; findings 1–6 remain
deferred. The test records this remaining behavior rather than claiming it fixed.

Covered **root DTZ bypasses TT score decoding**. With all three on and local
Syzygy, the mate example chooses legal g6g7 at clocks 0/98/99, Threads 1/4,
retaining the existing root cp 30743 output. That root score is not a DTM claim.
This verifies that boundary example, not every cursed/blessed win, rounded DTZ
boundary, zeroing sequence or root ranking. No DTZ implementation was changed.

## Reproduce

From the `v26rc` worktree, using cached Rust dependencies and existing lab files:

```powershell
py -3 validation/rc.py build
py -3 validation/rc.py parity
py -3 validation/rc.py repro
py -3 validation/rc.py speed
py -3 validation/rc.py speed4
```

Run sequentially. `build` invokes cargo build/test with
`--release --offline -j 4`, adding `--features frozen` for that variant;
`repro` explicitly runs the ignored real-table test. No frozen.options edits.
Executable hashes/options/FENs/net hash are retained in `rc-build.json` and the
parity/timing metadata. All logs and binaries are local ignored artifacts.
