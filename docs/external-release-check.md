# Release checks

Confirm a selected candidate on fresh neutral openings in independent self-play tests,
approximately 7,200 games at 8+0.08 and 2,400 at 40+0.4. Then run the fixed external
check against the current release. Freeze sample size before confirmation. An inconclusive
check holds the release pending a predeclared independent confirmation.

The external pool is `.github/xengine/pool-v1.json`. Its four equal-weight opponents,
source commits, exact network URLs, downloaded-file hashes, toolchain identity, build
commands, binary hashes and UCI declarations are locked. The `lock` workflow mode
prints an observed manifest and uploads a manifest-only artifact. Review and commit that
manifest before using `match` mode. Changes require a new planned pool version.
Matches fail closed on hash, compiler, CPU-feature or UCI-option mismatches. The cache
key includes the complete lock and prep script, covering sources, nets, toolchain and flags.

`tools/cloud-xengine-test.ps1` freezes each spec's network, options and full commit in
unique testkit assets. It dispatches, waits and saves `meta.txt`, `games.pgn` and
`results.json` under the sibling testing lab's `runs` folder. Reattach with `-RunId` and
the original identifying arguments. Final metadata comes from the archived request.
Schedules, per-game records and diagnostic logs are retained alongside the result.
Starts from locally archived successful external schedules are excluded from new requests.
Both Pawnstorm executables are built once in prep with pinned Rust and explicit
`-C target-cpu=x86-64-v3`, overriding per-commit native CPU settings. Their hashes
are frozen in the schedule and verified on every shard before play.
Example parameters for a 1,600-game calibration pilot are `-BlocksPerOpponentPerShard 25
-Jobs 4 -Seed 2026110703`. Both full code refs must already be published.

Each block contains both colours for candidate and release against one opponent, from
one normalized unique start. Opponents have disjoint starts. Both versions run on the
same standard Ubuntu runner. At most four game slots run per shard, bounded by CPU
count, with Threads 1, Hash 16, fresh processes, disabled books/learning/tablebases and
no evaluation adjudication. Candidate-first and release-first order alternates.
Eight shards with 125 blocks per opponent give 16,000 games. Production size must
be calibrated from pilot variance and throughput. A planned longer-TC pilot can assess
variance per runner-hour before freezing production size.

Analysis uses 10,000 paired opening-block bootstrap replicates, stratified by opponent,
with a recorded seed. It reports equal-weight pool-Elo difference, two-sided 95% CI,
raw scores, per-opponent WDL/covariance/variance, leave-one-opponent-out effects,
runner-cluster sensitivity and terminations. Boundary score cells yield an undefined
estimate and an inconclusive decision. Duplicate, incomplete or inconsistent blocks
invalidate the run. Engine crashes and illegal moves block release.

- A lower CI endpoint above zero gives evidence of improvement.
- A lower endpoint above −10 passes non-regression.
- An upper endpoint below −10 fails.
- Other outcomes remain inconclusive.

The dashboard displays these checks separately from ratings. External games and all
derived material are testing-only and must never enter training datasets or mined books.
Artifacts expire after three days. Download results before expiry. Only standard free
runners are used. Stop on GitHub capacity warnings, throttling or failures, and investigate
infrastructure failures before dispatching another run. The shared `test-match`
concurrency group serializes self-play and external checks.

The lab queue accepts `.xtest` files with exactly this pipe-separated schema.

```text
Name|Dev|DevRef|Base|BaseRef|TC|BlocksPerOpponentPerShard|Jobs|Pool|Seed|Book|Notes
```

Publish queue files using a temporary file followed by an atomic rename. The queue
claims them as `.running`, preserves the stop-file rule and stops on child failures.
The existing `.test` schema and behavior remain supported. A timeout stops the queue,
since the GitHub run may still be active. Inspect and reattach before restarting.
