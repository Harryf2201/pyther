//! Bounded streaming interleave; weights multiply remaining record counts.
use bullet_lib::value::loader::CanBeDirectlySequentiallyLoaded;
use bullet_trainer::reader::DataReader;
use rand::{Rng, SeedableRng, rngs::StdRng};
use std::{
    fs::File,
    io::{self, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
#[derive(Clone)]
pub struct TrainingData {
    parts: Vec<(PathBuf, f64, u64)>,
}
impl TrainingData {
    pub fn new(arg: &str) -> io::Result<Self> {
        let mut entries = Vec::new();
        if Path::new(arg)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("txt"))
        {
            let base = Path::new(arg).parent().unwrap_or(Path::new("."));
            for line in std::fs::read_to_string(arg)?.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let (path, weight) = match line.rsplit_once(char::is_whitespace) {
                    Some((p, w)) if w.parse::<f64>().is_ok() => {
                        (p.trim(), w.parse::<f64>().unwrap())
                    }
                    _ => (line, 1.0),
                };
                let path = Path::new(path);
                entries.push((
                    if path.is_absolute() {
                        path.to_owned()
                    } else {
                        base.join(path)
                    },
                    weight,
                ));
            }
        } else {
            entries.extend(arg.split(',').map(|p| (PathBuf::from(p.trim()), 1.0)));
        }
        if entries.is_empty() {
            return Err(io::Error::other("empty data list"));
        }
        let mut parts = Vec::new();
        for (path, weight) in entries {
            if !weight.is_finite() || weight <= 0.0 {
                return Err(io::Error::other("weights must be finite and positive"));
            }
            let bytes = std::fs::metadata(&path)?.len();
            if bytes == 0 || bytes % 32 != 0 {
                return Err(io::Error::other(format!(
                    "{}: expected nonempty 32-byte records",
                    path.display()
                )));
            }
            parts.push((path, weight, bytes / 32));
        }
        let max = parts.iter().map(|p| p.1).fold(0.0, f64::max);
        for p in &mut parts {
            p.1 /= max;
            if p.1 == 0.0 {
                return Err(io::Error::other("weight range too large"));
            }
        }
        if parts
            .iter()
            .try_fold(0u64, |sum, p| sum.checked_add(p.2))
            .is_none()
        {
            return Err(io::Error::other("total record count overflows u64"));
        }
        Ok(Self { parts })
    }
    pub fn positions(&self) -> u64 {
        self.parts.iter().map(|p| p.2).sum()
    }
}
impl<T: CanBeDirectlySequentiallyLoaded> DataReader<T> for TrainingData {
    fn read_chunks<F: FnMut(&[T]) -> bool>(&self, skip_count: usize, mut f: F) {
        assert_eq!(size_of::<T>(), 32);
        let total = self.positions();
        let mut epoch = skip_count as u64 / total;
        let mut skip = skip_count as u64 % total;
        let mut readers: Vec<_> = self
            .parts
            .iter()
            .map(|p| BufReader::with_capacity(65536, File::open(&p.0).expect("open training part")))
            .collect();
        let mut chunk: Vec<T> = Vec::with_capacity(16384);
        loop {
            // Catch growth as well as truncation, including between epochs. Exact
            // reads below also detect truncation after this metadata check.
            for (reader, part) in readers.iter().zip(&self.parts) {
                assert_eq!(
                    reader
                        .get_ref()
                        .metadata()
                        .expect("stat training part")
                        .len(),
                    part.2 * 32,
                    "training part size changed: {}",
                    part.0.display()
                );
            }
            if self.parts.len() == 1 {
                let reader = &mut readers[0];
                reader
                    .seek(SeekFrom::Start(skip * 32))
                    .expect("seek training part");
                let mut left = total - skip;
                skip = 0;
                while left > 0 {
                    let count = left.min(16384) as usize;
                    // Safety: the marker trait allows all byte patterns. Set the
                    // length only after read_exact has initialized every byte.
                    let bytes = unsafe {
                        std::slice::from_raw_parts_mut(chunk.as_mut_ptr().cast::<u8>(), count * 32)
                    };
                    reader
                        .read_exact(bytes)
                        .expect("read training records (part changed or I/O failure)");
                    unsafe {
                        chunk.set_len(count);
                    }
                    if f(&chunk) {
                        return;
                    }
                    chunk.clear();
                    left -= count as u64;
                }
                epoch = epoch.wrapping_add(1);
                continue;
            }
            let mut rng = StdRng::seed_from_u64(0x5041574e53544f52u64.wrapping_add(epoch));
            let mut remaining: Vec<_> = self.parts.iter().map(|p| p.2).collect();
            // Recompute ancestors from their children rather than subtracting
            // floating-point deltas: exhausted subtrees become exactly zero.
            let base = self.parts.len().next_power_of_two();
            let mut tree = vec![0.0; base * 2];
            for (i, part) in self.parts.iter().enumerate() {
                tree[base + i] = part.2 as f64 * part.1;
            }
            for i in (1..base).rev() {
                tree[i] = tree[2 * i] + tree[2 * i + 1];
            }
            for reader in &mut readers {
                reader
                    .seek(SeekFrom::Start(0))
                    .expect("rewind training part");
            }
            for _ in 0..total {
                let mut pick = rng.random::<f64>() * tree[1];
                let mut node = 1;
                while node < base {
                    let left = node * 2;
                    // Also handles pick rounding up to the total mass: never
                    // descend into an empty subtree (including padding leaves).
                    if tree[left + 1] == 0.0 || pick < tree[left] {
                        node = left;
                    } else {
                        pick -= tree[left];
                        node = left + 1;
                    }
                }
                let index = node - base;
                remaining[index] -= 1;
                tree[node] = remaining[index] as f64 * self.parts[index].1;
                while node > 1 {
                    node /= 2;
                    tree[node] = tree[node * 2] + tree[node * 2 + 1];
                }
                let mut record = std::mem::MaybeUninit::<T>::zeroed();
                // Safety: Bullet's marker trait guarantees every byte pattern is valid T.
                let bytes = unsafe {
                    std::slice::from_raw_parts_mut(record.as_mut_ptr().cast::<u8>(), size_of::<T>())
                };
                readers[index]
                    .read_exact(bytes)
                    .expect("read training record (part changed or I/O failure)");
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                chunk.push(unsafe { record.assume_init() });
                if chunk.len() == 16384 {
                    if f(&chunk) {
                        return;
                    }
                    chunk.clear();
                }
            }
            if !chunk.is_empty() {
                if f(&chunk) {
                    return;
                }
                chunk.clear();
            }
            epoch = epoch.wrapping_add(1);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Clone, Copy)]
    struct Record([u8; 32]);
    unsafe impl CanBeDirectlySequentiallyLoaded for Record {}
    #[test]
    fn tree_matches_weighted_scan() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("multiload-tree-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let counts = [13u64, 1, 37, 4, 19, 2, 61];
        let weights = [1.0, 0.25, 0.5, 1.0, 0.25, 0.5, 1.0];
        let mut manifest = String::new();
        for (i, &count) in counts.iter().enumerate() {
            std::fs::write(
                dir.join(format!("{i}.bin")),
                vec![i as u8; count as usize * 32],
            )
            .unwrap();
            manifest.push_str(&format!("{i}.bin {}\n", weights[i]));
        }
        std::fs::write(dir.join("parts.txt"), manifest).unwrap();
        let loader = TrainingData::new(dir.join("parts.txt").to_str().unwrap()).unwrap();
        let mut expected = Vec::new();
        for epoch in 0..3 {
            let mut rng = StdRng::seed_from_u64(0x5041574e53544f52u64 + epoch);
            let mut remaining = counts;
            for _ in 0..loader.positions() {
                let mass: f64 = remaining
                    .iter()
                    .zip(weights)
                    .map(|(&n, w)| n as f64 * w)
                    .sum();
                let mut pick = rng.random::<f64>() * mass;
                let index = remaining
                    .iter()
                    .zip(weights)
                    .position(|(&n, w)| {
                        let mass = n as f64 * w;
                        if pick < mass {
                            true
                        } else {
                            pick -= mass;
                            false
                        }
                    })
                    .unwrap();
                remaining[index] -= 1;
                expected.push(index as u8);
            }
            assert_eq!(remaining, [0; 7]);
        }
        let mut actual = Vec::new();
        loader.read_chunks(0, |r: &[Record]| {
            actual.extend(r.iter().map(|r| r.0[0]));
            actual.len() == expected.len()
        });
        assert_eq!(actual, expected);
        // Very different positive weights must still drain every leaf exactly once.
        let mut extreme = loader.clone();
        extreme.parts[0].1 = 1e-300;
        let mut observed = [0u64; 7];
        extreme.read_chunks(0, |r: &[Record]| {
            for record in r {
                observed[record.0[0] as usize] += 1;
            }
            true
        });
        assert_eq!(observed, counts);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn resume_boundaries_and_changed_files() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("multiload-check-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, range) in [("a.bin", 0u32..17000), ("b.bin", 17000..20003)] {
            let data: Vec<u8> = range
                .flat_map(|i| {
                    let mut bytes = [0; 32];
                    bytes[..4].copy_from_slice(&i.to_le_bytes());
                    bytes
                })
                .collect();
            std::fs::write(dir.join(name), data).unwrap();
        }
        for paths in [
            dir.join("a.bin").display().to_string(),
            format!(
                "{},{}",
                dir.join("a.bin").display(),
                dir.join("b.bin").display()
            ),
        ] {
            let loader = TrainingData::new(&paths).unwrap();
            let total = loader.positions() as usize;
            let collect = |skip, wanted| {
                let mut result = Vec::new();
                loader.read_chunks(skip, |r: &[Record]| {
                    result.extend(
                        r.iter()
                            .take(wanted - result.len())
                            .map(|r| u32::from_le_bytes(r.0[..4].try_into().unwrap())),
                    );
                    result.len() == wanted
                });
                result
            };
            let stream = collect(0, total * 3 + 31);
            for skip in [
                0,
                1,
                16383,
                16384,
                total - 1,
                total,
                total + 1,
                total * 2 + 19,
            ] {
                assert_eq!(
                    collect(skip, total / 2),
                    stream[skip..skip + total / 2],
                    "offset {skip}"
                );
            }
            // Truncation during a callback must panic at the next exact read,
            // even if BufReader has already prefetched some records.
            let result = std::panic::catch_unwind(|| {
                loader.read_chunks(0, |_: &[Record]| {
                    File::options()
                        .write(true)
                        .open(dir.join("a.bin"))
                        .unwrap()
                        .set_len(0)
                        .unwrap();
                    false
                });
            });
            assert!(result.is_err());
            // Restore for the multi-file iteration.
            std::fs::write(dir.join("a.bin"), vec![0; 17000 * 32]).unwrap();
        }
        for len in [0, 31, 33] {
            std::fs::write(dir.join("bad.bin"), vec![0; len]).unwrap();
            assert!(TrainingData::new(dir.join("bad.bin").to_str().unwrap()).is_err());
        }
        let loader = TrainingData::new(dir.join("a.bin").to_str().unwrap()).unwrap();
        std::fs::write(dir.join("a.bin"), [0; 32]).unwrap();
        assert!(std::panic::catch_unwind(|| loader.read_chunks(0, |_: &[Record]| true)).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    #[ignore = "release throughput benchmark; creates and removes 280 MiB under target"]
    fn throughput() {
        assert!(!cfg!(debug_assertions), "run with --release");
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("multiload-bench-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let _cleanup = Cleanup(dir.clone());
        let unit = vec![7u8; 4 * 1024 * 1024];
        let mut paths = Vec::new();
        for i in 0..31 {
            use std::io::Write;
            let path = dir.join(format!("{i}.bin"));
            let mut file = File::create(&path).unwrap();
            for _ in 0..if i == 0 { 40 } else { 1 } {
                file.write_all(&unit).unwrap();
            }
            paths.push(path.to_str().unwrap().to_owned());
        }
        let loader = TrainingData::new(&paths.join(",")).unwrap();
        for run in 0..4 {
            let start = std::time::Instant::now();
            let mut count = 0u64;
            loader.read_chunks(0, |r: &[Record]| {
                std::hint::black_box(r);
                count += r.len() as u64;
                count == loader.positions()
            });
            eprintln!(
                "run {run}: {count} records / {:.6}s = {:.2} M records/s",
                start.elapsed().as_secs_f64(),
                count as f64 / start.elapsed().as_secs_f64() / 1e6
            );
        }
    }
    #[test]
    fn epochs_weights_and_resume() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("pawnstorm-multiload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, range) in [("a.bin", 0..100), ("b.bin", 100..150)] {
            let data: Vec<u8> = range.flat_map(|i| [i; 32]).collect();
            std::fs::write(dir.join(name), data).unwrap();
        }
        for weight in [1, 10] {
            std::fs::write(dir.join("parts.txt"), format!("a.bin {weight}\nb.bin 1\n")).unwrap();
            let loader = TrainingData::new(dir.join("parts.txt").to_str().unwrap()).unwrap();
            assert_eq!(loader.positions(), 150);
            let mut epochs = Vec::new();
            loader.read_chunks(0, |r: &[Record]| {
                epochs.push(r.iter().map(|r| r.0[0]).collect::<Vec<_>>());
                epochs.len() == 2
            });
            for e in &epochs {
                assert_eq!(e.iter().filter(|&&n| n < 100).count(), 100);
                let mut sorted = e.clone();
                sorted.sort();
                assert_eq!(sorted, (0..150).collect::<Vec<u8>>());
            }
            let mut resumed = Vec::new();
            loader.read_chunks(167, |r: &[Record]| {
                resumed.extend(r.iter().map(|r| r.0[0]));
                true
            });
            assert_eq!(resumed, epochs[1][17..]);
            if weight == 10 {
                assert!(epochs[0][..50].iter().filter(|&&n| n < 100).count() > 40);
            }
        }
        let comma = TrainingData::new(&format!(
            "{},{}",
            dir.join("a.bin").display(),
            dir.join("b.bin").display()
        ))
        .unwrap();
        assert_eq!(comma.positions(), 150);
        let single = TrainingData::new(dir.join("a.bin").to_str().unwrap()).unwrap();
        let mut first = Vec::new();
        single.read_chunks(17, |r: &[Record]| {
            first.extend(r.iter().map(|r| r.0[0]));
            true
        });
        assert_eq!(first, (17..100).collect::<Vec<u8>>());
        for text in [
            "",
            "a.bin 0",
            "a.bin -1",
            "a.bin NaN",
            "a.bin inf",
            "missing.bin",
        ] {
            std::fs::write(dir.join("parts.txt"), text).unwrap();
            assert!(
                TrainingData::new(dir.join("parts.txt").to_str().unwrap()).is_err(),
                "{text}"
            );
        }
        std::fs::write(dir.join("bad.bin"), [0; 33]).unwrap();
        assert!(TrainingData::new(dir.join("bad.bin").to_str().unwrap()).is_err());
        // Exercise multiple output chunks, per-file buffer refills and the epoch tail.
        for (name, range) in [("a.bin", 0u32..20000), ("b.bin", 20000..40000)] {
            let data: Vec<u8> = range
                .flat_map(|i| {
                    let mut record = [0; 32];
                    record[..4].copy_from_slice(&i.to_le_bytes());
                    record
                })
                .collect();
            std::fs::write(dir.join(name), data).unwrap();
        }
        let large = TrainingData::new(&format!(
            "{},{}",
            dir.join("a.bin").display(),
            dir.join("b.bin").display()
        ))
        .unwrap();
        let mut records = Vec::new();
        let mut lengths = Vec::new();
        large.read_chunks(0, |r: &[Record]| {
            lengths.push(r.len());
            records.extend(
                r.iter()
                    .map(|v| u32::from_le_bytes(v.0[..4].try_into().unwrap())),
            );
            records.len() == 80000
        });
        assert_eq!(lengths, [16384, 16384, 7232, 16384, 16384, 7232]);
        for epoch in records.chunks_exact(40000) {
            let mut sorted = epoch.to_vec();
            sorted.sort();
            assert_eq!(sorted, (0..40000).collect::<Vec<u32>>());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
