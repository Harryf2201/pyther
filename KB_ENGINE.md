# Unmirrored KB4 engine port

Based on xsearch 94f4385. `kbengine` defaults to the unchanged plain 1024 input
architecture; enable Cargo feature `kb4nm` for the trainer's `mlkb4nm` network.
`kbengine-cloud` uses `default = ["kb4nm"]` for default-feature cloud builds.
No new UCI option or search tuning is involved. Use `--no-default-features` for
plain mode regardless of branch.

## Build

Run in this worktree, below-normal priority, with the lab health control released:

```powershell
(Get-Process -Id $PID).PriorityClass = 'BelowNormal'
$env:CARGO_TARGET_DIR = "$PWD\target"
cargo build --release -p pawnstorm --features kb4nm -j 4 --offline
```

The worktree's existing Cargo config uses target-cpu=native. For a portable x86
AVX2-only build (no BMI2/PEXT):

```powershell
$env:RUSTFLAGS = '-C target-cpu=x86-64 -C target-feature=+avx2'
$env:CARGO_TARGET_DIR = "$PWD\target-avx2"
cargo build --release -p pawnstorm --features kb4nm -j 4 --offline
```

Diagnostic `kb-refresh-only` implies `kb4nm` and rebuilds a changed king zone
from scratch. The normal `kb4nm` build uses Finny. Both share identical NNUE
kernels, static evaluation and search behaviour. Plain builds allocate no cache
and compile out the child-board argument and bucket calculations.

## Network contract

FT is 3072 x 1024 i16, shared 1024 bias, QA255. Pairwise products and all eight
material-selected 16/32/1 layers/kernels are retained from xsearch. The trainer's
factoriser and residuals are merged into the exported FT; inference never loads
or adds a factoriser separately.

Own-king zones are 0 for back-rank a/b/g/h; 1 for back-rank c/d/e/f; 2 for the
second rank; 3 for all other ranks. Black rank-flips by ^56. Piece squares never
file-mirror. Each Searcher/thread owns eight Finny entries (perspective x zone),
each holding an accumulator and twelve absolute piece bitboards. King-zone
changes rebuild just the moved king's perspective via the cache; other moves
retain fused incremental updates. Root refreshes and null-move accumulator
copies retain xsearch's existing behaviour. The network generation invalidates
old cached weights after any successful network replacement.

File sizes must equal the 64-byte-padded export exactly: **6,836,800 bytes KB**,
**2,118,208 bytes plain**. Truncated, oversized and wrong-width files are rejected;
nonfinite downstream floats are also rejected. Failed loads leave the previous
active network unchanged.

Mirrored 1024 KB4 has the same byte size as unmirrored KB4. External `EvalFile`
therefore **requires** an explicit architecture marker, checked before activation.
Candidates in priority order: `<network-path>.architecture.txt`, `architecture.txt`
in the network's directory, then `architecture.txt` in that directory's parent.
The trainer's checkpoint parent marker works directly. A closer conflicting
marker rejects the file, rather than being bypassed by a farther valid marker.
Its contents are (LF or CRLF accepted):

```
inputs=mlkb4nm
hidden=1024
output_buckets=8
residual_scale=0.25
mirrored=false
```

When copying a checkpoint quantised.bin into a standalone .nnue, copy its marker
as `<name>.nnue.architecture.txt`. Do not relabel a mirrored network to bypass
validation. A marker is a provenance assertion, not a checksum binding.

**Embedded-file limitation:** cloud builders replace `nets/default.bin` before
building. That existing pipeline does not supply an architecture header/manifest.
Embedded exports are checked for exact size and finite downstream floats, but
the `kb4nm` compile-time contract asserts their unmirrored provenance. A valid-size
mirrored embedded network cannot be identified from weights alone. The engine
prints an explicit warning about this assumption when accepting such a headerless
embedded KB export. Claude must choose the correct training artifact. An embedded
plain net is rejected in KB builds with a diagnostic; load a valid EvalFile before
searching. No workflow or release tooling has been changed here.

## Verification

Run tests single-threaded because the pre-existing test network is global:

```powershell
cargo test --release -p pawnstorm --lib --features kb4nm -j 4 --offline -- --test-threads=1 --nocapture
cargo test --release -p pawnstorm --lib --features kb-refresh-only -j 4 --offline -- --test-threads=1 --nocapture
cargo test --release -p pawnstorm --lib --no-default-features -j 4 --offline -- --test-threads=1 --nocapture
```

Tests compare incremental updates with full refresh over hundreds of random
games, forced special moves, sibling cache revisits, null copies and subsequent
real moves, all king squares from both perspectives, and network-generation
invalidation. The existing search regression tests remain enabled. Lab scripts,
raw results, float comparison, plain node parity and paired timing are under
`testing/coord/kb-audit/engine-port/` alongside the final report.

Local evidence (smoke net, not the full trained net): all 12 bench positions at
depth 12 match xsearch5 with x25-big7 net/options, 275815 nodes each engine,
including exact scores/best moves. KB verification checked 46648 random moves,
special moves, null paths, 7479 zone changes, 840 exhaustive king moves and cache
generation invalidation. Native/AVX2-only/scalar smoke inference is identical
on 4000 FENs; against CPU float inference mean error is .303 cp, p95 .840 cp.
The 300-batch smoke net is nearly flat: float outputs are only -.40..+.84 cp
and every integer engine output is zero. Its small error alone is weak evidence.
A second CPU-only diagnostic lifts our mature plain big7 net into F, repeats its
FT into all four zones with R=0, and changes no source network. All 4000 resulting
KB engine outputs exactly equal the plain engine (1885 distinct cp values),
with 11.076 cp mean/34.075 cp p95 error versus float. A separate 98304-case test
checks engine feature indices against the explicit unmirrored trainer contract.

Six alternating paired depth-24 suites (12 FENs each, 3341390 nodes per suite)
gave Finny/full-refresh geometric NPS ratios 1.028 native and 1.038 AVX2-only.
Approximate paired 95% intervals were .951..1.112 and .953..1.131 respectively:
local contention prevents claiming a reliable gain. These are throughput tests,
not playing-strength tests; see the lab report for all ranges and methodology.
The smoke net's nearly constant output also limits extrapolation of its search
trees and cache traffic to the fully trained bucketed net.

## ARM checks Claude must perform

* Build `kbengine-cloud` default features on ARM and confirm KB is active. Verify
  it embedded the unmirrored 1024 export, not the same-size old mirrored export.
* Run single-threaded tests and the same FEN inference comparison. Compare nodes
  between refresh-only and Finny builds on identical nets/options. Test both NEON
  widening and dotprod configurations used by deployment where available.
* Measure NPS and per-thread cache/memory impact at deployment thread count;
  validate Threads>1, repeated games, and EvalFile A/B/A switches. Existing NEON
  pairwise/dot kernels are unchanged, but they were not executed on this PC.
* Use the full trained net for held-out and fixed-node quality checks before
  fixed-time cloud games. This port's smoke-net NPS and quantisation agreement do
  not predict playing strength or the full net's cache locality.
