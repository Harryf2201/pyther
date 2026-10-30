# Pyther

A UCI chess engine written in Rust (formerly called Pawnstorm; the crate, binaries and lab files still use that name).

- Bitboard move generation with PEXT slider attacks (perft-verified)
- Principal variation search with transposition table, null-move pruning, late move reductions,
  singular extensions, history/continuation-history move ordering and more
- NNUE evaluation `(768 -> 1024)x2 pairwise -> 16 -> 32 -> 1` with 8 output buckets, trained with [bullet](https://github.com/jw1912/bullet)
  on self-play data, with a hand-crafted fallback evaluation

See [EXPERIMENTS.md](EXPERIMENTS.md) for everything tried so far and where each result lives.

## Building

```
cargo build --release
./target/release/pawnstorm          # UCI mode, for use in any chess GUI
./target/release/pawnstorm bench    # speed test
./target/release/pawnstorm perft    # move generation correctness test
```

For a build with fixed search parameters, use `cargo build --release --offline -j 4
--features frozen`. `frozen.options` contains `Name=value` overrides; parameters
omitted from that file keep their defaults in `src/param_defs.rs`. The build
rejects unknown names, malformed values and values outside the declared bounds.
Changing the file triggers regeneration. The feature is off by default.

Frozen builds keep advertising the usual UCI options. Setting a parameter to its
compiled value is silent; any other value is ignored with an `info string`
message naming the compiled value. Hash, Threads, EvalFile and other non-parameter
options work as usual. Rebuild to change a frozen parameter. Local validation
commands and ARM instructions are in `validation/report.md`.

## Training data

`pawnstorm datagen <threads> <positions> <soft-nodes> <out-dir> [seconds] [seed]` plays fast
self-play games and writes `<fen> | <score> | <result>` lines. The `datagen` GitHub Actions
workflow runs the same generator on hosted runners.

## Layout

- `src/` engine
- `trainer/` NNUE trainer (needs CUDA)
- `tools/` local tooling: tournament launcher, dashboard, data preparation, hardware watchdog
