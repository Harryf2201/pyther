# Streaming training parts

`tools/train.ps1 -Name net-next -Data 'gen5,gen6,gen7'` resolves each name
in the lab's `training/data` folder, appending `.bin` when needed. Single names
keep the existing behavior. Alternatively use `-Data parts.txt` (a full path or
a filename in `training/data`). The trainer executable itself accepts a single
`.bin`, comma-separated paths, or a `.txt` list:

```text
# Relative paths are relative to this list, including paths containing spaces.
gen5.bin
gen6.bin 1.0
parts/gen7.bin 2.0
```

Blank lines and whole-line `#` comments are ignored. Weights must be finite and
positive; default is 1. Selection probability is proportional to
`weight * remaining_positions`. Every record is used exactly once per epoch,
then all parts rewind. Weights therefore bias ordering within an epoch, **not**
the total epoch proportions; construct/sample parts to change total proportions.
With equal weights this is a uniform random interleave preserving each part's
internal order, equivalent to a random merge of already shuffled parts.

The reader implements Bullet's `DataReader` in this crate; Bullet is unchanged.
A single file uses a checked sequential reader with direct seeking on resume. Multi-file ordering
is deterministic from the epoch number, with a different seed each epoch, so
resume offsets reproduce the stream with the same files/order/weights and RNG
version and selection implementation. The sum tree can round weighted sums differently
from the original linear scan, so old scan-based checkpoints are not guaranteed
to reproduce the identical ordering after this change. Resuming replays the current epoch prefix; this can be slow for large
offsets. Input files must remain unchanged during training.

Memory is 64 KiB per open part plus a 512 KiB output chunk and O(files) metadata,
in addition to Bullet's existing queues/batches. All files stay open; there is
no fixed file-count cap, but OS handle limits apply and selection costs O(log files)
per position. Reads are buffered and multiplexed by one loader thread, with no
per-file worker threads. Files must be nonempty and a multiple of 32 bytes;
I/O failures abort rather than silently dropping data. Commas cannot occur in
paths in comma-list mode (use a list file for those paths).

Selection uses a binary sum tree with leaves `weight * remaining_positions`.
Updating ancestors from their children avoids subtractive drift and makes
exhausted subtrees exactly zero. Partial chunks are emitted at epoch boundaries;
Bullet carries them into the next batch. `skip_count` is an absolute record offset
(Bullet passes batch size times completed batches), not a batch or chunk count.
Size changes are checked on opened handles every epoch; exact reads detect
unexpected EOF and read failures. Same-size content changes are not detected.

## Throughput check (Oct 26)

Run `cargo test --release --manifest-path trainer/Cargo.toml throughput -- --ignored --nocapture`.
The ignored benchmark creates 31 synthetic files under `trainer/target`: one
160 MiB file and thirty 4 MiB files (280 MiB / 9,175,040 records total), all
equal weight, then consumes four epochs independently. Creation is untimed;
each timing includes open, allocation, selection, reads and callback delivery.
The callback black-boxes the chunk; this measures loading without GPU mapping.
Fixtures are removed automatically, including on unwinding.

Pinned to logical CPU 0, BelowNormal priority, one reader thread, with the
data-preparation job running: original scan 23.22-24.34M records/s (four runs);
sum tree repeat 33.96, 32.84, 32.02, 32.76M records/s. The initial tree run
had a 17.03M first sample followed by 32.21-34.23M; the repeat above checked
that transient. Unpinned original baseline: 27.65-28.37M. These small files
are cache-resident and do not establish sustained throughput from cold storage.

To reproduce the pinned run after building tests, launch the test executable
under `trainer/target/release/deps` with `throughput --ignored --nocapture`,
set its `ProcessorAffinity` to `[IntPtr]1` and `PriorityClass` to `BelowNormal`
immediately via PowerShell `Start-Process -WindowStyle Hidden -PassThru`, then
wait for exit. CPU scheduling and concurrent preparation affect timings.

Default batch settings remain 16384 x 6104. Optional executable arguments after
inputs, `batch-size=N batches=N`, support short diagnostics. `val=...` is passed
through unchanged; this Bullet checkout currently warns that validation is not
implemented, so it does not report validation loss.

Build/test with CUDA 13.3 as in `tools/train.ps1`:

```powershell
cargo build --release --manifest-path trainer/Cargo.toml
cargo test --release --manifest-path trainer/Cargo.toml
powershell -NoProfile -ExecutionPolicy Bypass -File tools/smoke-multiload.ps1
```

The smoke script uses two consecutive 65536-position slices of `val-gen6.bin`,
1 superbatch of 1024 x 1024 positions, a small 128-wide multi-layer net, and
checks GPU identification, successful exit and decreasing logged training loss.
Artifacts stay under `trainer/target/multiload-smoke-*`.
