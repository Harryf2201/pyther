//! Deterministic, single-threaded microbenchmarks. Counters are opt-in and thread-local.
//! Timings are inclusive; nested rows must not be added to their parents.
use crate::{bench::BENCH_FENS, board::Board, nnue, search::{Limits, Searcher}, tt::{TT, BOUND_EXACT}, types::*};
use std::{hint::black_box, sync::{Arc, atomic::AtomicBool}, time::Instant};

pub const NAMES: [&str; 15] = ["refresh", "update.quiet", "update.capture", "update.castle",
    "update.promotion", "pairwise.side", "l1.dense", "l1.sparse", "l2+l3", "evaluate",
    "make_move", "movegen.pseudo", "movegen.legal", "see", "tt.probe"];
pub const STORE: usize = 15;
#[cfg(feature = "profile-counters")]
thread_local! {
    static COUNTS: [std::cell::Cell<u64>; 16] = const { [const { std::cell::Cell::new(0) }; 16] };
    static ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
#[cfg(feature = "profile-counters")]
#[inline(always)]
pub fn hit(id: usize) {
    if ENABLED.with(|e| e.get()) {
        COUNTS.with(|c| c[id].set(c[id].get() + 1));
    }
}
// Macro expands to nothing in normal builds, including argument evaluation.
#[macro_export]
macro_rules! profile_hit {
    ($id:expr) => { #[cfg(feature = "profile-counters")] $crate::profile::hit($id); };
}

pub(crate) fn time<T>(name: &str, n: usize, mut f: impl FnMut(usize) -> T) -> f64 {
    for i in 0..n.min(4096) { black_box(f(black_box(i))); }
    // Median of five batches; black_box prevents constant-folding and dead stores.
    let mut samples = [0.0; 5];
    for t in &mut samples {
        let start = Instant::now();
        for i in 0..n { black_box(f(black_box(i))); }
        *t = start.elapsed().as_secs_f64() * 1e9 / n as f64;
    }
    samples.sort_by(f64::total_cmp);
    println!("micro {name:20} {:10.2} ns/op [{:.2}, {:.2}]", samples[2], samples[0], samples[4]);
    samples[2]
}

pub fn run(args: &[String]) {
    let n = args.first().map(|s| s.parse::<usize>().expect("iterations")).unwrap_or(20000).max(1);
    let depth = args.get(1).map(|s| s.parse::<i32>().expect("depth")).unwrap_or(12);
    assert!((1..=64).contains(&depth));
    if let Some(path) = args.get(2) { nnue::load_file(path).expect("EvalFile"); }
    if let Some(path) = args.get(3) {
        for line in std::fs::read_to_string(path).expect("options file").lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            let (name, value) = line.split_once('=').expect("Name=value");
            let value = value.trim().parse().expect("option value");
            assert!(crate::params::set(name.trim(), value), "unknown option {}", name.trim());
        }
    }
    assert!(nnue::active(), "profile requires a loaded network");
    println!("profile arch={} dotprod={} avx2={} avxvnni={} frozen={} counters={} n={n} depth={depth}",
        std::env::consts::ARCH, cfg!(target_feature="dotprod"), cfg!(target_feature="avx2"),
        cfg!(target_feature="avxvnni"), cfg!(feature="frozen"), cfg!(feature="profile-counters"));
    let boards: Vec<_> = BENCH_FENS.iter().map(|fen| Board::from_fen(fen).unwrap()).collect();
    let mut moves: Vec<_> = boards.iter().flat_map(|b| b.legal_moves().into_iter().map(move |mv| (*b, mv))).collect();
    let ep = Board::from_fen("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1").unwrap();
    moves.push((ep, ep.parse_move("e5d6").unwrap()));
    let mut costs = nnue::profiling::measure(n, &boards, &moves);
    costs[10] = time(NAMES[10], n, |i| { let (b, mv) = black_box(&moves[i % moves.len()]); let mut c = *b; c.make_move(*mv); c });
    costs[11] = time(NAMES[11], n, |i| { let mut list = MoveList::new(); black_box(&boards[i % boards.len()]).gen_moves(&mut list, false); list });
    costs[12] = time(NAMES[12], n, |i| black_box(&boards[i % boards.len()]).legal_moves());
    costs[13] = time(NAMES[13], n, |i| { let (b, mv) = black_box(&moves[i % moves.len()]); b.see(*mv, black_box(0)) });
    let tt = Arc::new(TT::new(16));
    for (b, mv) in &moves { tt.store(b.hash, *mv, 12, 5, 8, BOUND_EXACT, false); }
    costs[14] = time(NAMES[14], n, |i| tt.probe(black_box(moves[i % moves.len()].0.hash)));
    let store = time("tt.store", n, |i| { let (b, mv) = &moves[i % moves.len()]; tt.store(black_box(b.hash), *mv, 12, 5, 8, BOUND_EXACT, false) });
    println!("note make_move includes Board copy; legal movegen includes allocation/filtering; SEE threshold=0; TT is warm/hit-dominated");
    let mut s = Box::new(Searcher::new(tt.clone(), Arc::new(AtomicBool::new(false))));
    s.print = false;
    let mut nodes = 0;
    let mut seconds = 0.0;
    #[cfg(feature = "profile-counters")]
    COUNTS.with(|c| { for v in c { v.set(0); } });
    for (i, b) in boards.iter().enumerate() {
        tt.clear(); s.clear(); s.hashes.clear(); s.hashes.push(b.hash);
        #[cfg(feature = "profile-counters")]
        ENABLED.with(|e| e.set(true));
        let start = Instant::now();
        let (mv, score) = s.search(b, &Limits { depth: Some(depth), ..Default::default() });
        seconds += start.elapsed().as_secs_f64(); nodes += s.nodes;
        #[cfg(feature = "profile-counters")]
        ENABLED.with(|e| e.set(false));
        println!("position {i:02} nodes={} score={score} best={mv}", s.nodes);
    }
    println!("search nodes={nodes} seconds={seconds:.6} nps={:.0}", nodes as f64 / seconds);
    #[cfg(feature = "profile-counters")]
    {
        println!("shares: inclusive estimates against instrumented search time; evaluate contains pairwise/L1/tail; legal contains pseudo; do not sum nested rows");
        let counts: [u64; 16] = COUNTS.with(|c| std::array::from_fn(|i| c[i].get()));
        for id in 0..16 {
            let (name, cost) = if id == STORE { ("tt.store", store) } else { (NAMES[id], costs[id]) };
            let estimate = counts[id] as f64 * cost * 1e-9;
            println!("share {name:20} calls={:12} estimated_ms={:10.3} percent={:6.2}", counts[id], estimate * 1e3, estimate / seconds * 100.0);
        }
    }
    #[cfg(not(feature = "profile-counters"))]
    { black_box((costs, store)); println!("build with --features profile-counters for call counts and estimated shares"); }
}
