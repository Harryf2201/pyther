use super::*;

#[test]
fn tb_band_round_trips_and_preserves_distance_order() {
    assert_eq!((TB_BOUND, TB_WIN, MATE_BOUND), (30615, 30743, 30872));
    for sign in [-1, 1] {
        for store_ply in [0, 3, 7, MAX_PLY - 1] {
            // A real search score includes the root-to-node ply and then the
            // distance to the TB result. Their sum fits the search horizon.
            for distance in [0, 1, MAX_PLY - store_ply] {
                let score = sign * (TB_WIN - store_ply as i32 - distance as i32);
                let stored = score_to_tt_with(score, store_ply, true);
                assert_eq!(stored, sign * (TB_WIN - distance as i32));
                assert!(stored.abs() < MATE_BOUND);
                assert_eq!(score_from_tt_with(stored, store_ply, 0, true, false), score);
                for read_ply in [0, 3, 7] {
                    if read_ply + distance <= MAX_PLY {
                        assert_eq!(score_from_tt_with(stored, read_ply, 0, true, false),
                            sign * (TB_WIN - read_ply as i32 - distance as i32));
                    }
                }
                assert_eq!(score_to_tt_with(score, store_ply, false), score);
            }
        }
    }
    assert!(score_from_tt_with(TB_WIN - 2, 7, 0, true, false)
        > score_from_tt_with(TB_WIN - 3, 7, 0, true, false));
    // The complete declared band uses the same conversion as mates, even
    // though actual WDL probes never generate scores above TB_WIN - ply.
    for score in [TB_BOUND, TB_WIN, MATE_BOUND - 1, MATE_BOUND, MATE - 1] {
        for sign in [-1, 1] {
            let s = sign * score;
            assert_eq!(score_from_tt_with(score_to_tt_with(s, 7, true), 7, 0, true, false), s);
        }
    }
}

#[test]
fn r50_guard_boundaries_and_ordinary_scores() {
    for tb_ply in [false, true] {
        for sign in [-1, 1] {
            for ply in [0, 7] {
                for halfmove in [0, 90, 99, 100, 101, u16::MAX] {
                    let left = 100_u16.saturating_sub(halfmove) as i32;
                    for base in [MATE, TB_WIN] {
                        for distance in [(left - 1).max(0), left, left + 1] {
                            let score = sign * (base - ply as i32 - distance);
                            let stored = score_to_tt_with(score, ply, tb_ply);
                            let result = score_from_tt_with(stored, ply, halfmove, tb_ply, true);
                            assert_eq!(result, if distance > left { sign * (TB_BOUND - 1) } else { score },
                                "base={base} distance={distance} left={left} ply={ply} tb_ply={tb_ply}");
                            assert_eq!(score_from_tt_with(stored, ply, halfmove, tb_ply, false), score);
                        }
                    }
                }
            }
            for score in [0, 1, 1200, TB_BOUND - 1] {
                for ply in [0, 7, MAX_PLY - 1] {
                    let s = score * sign;
                    assert_eq!(score_to_tt_with(s, ply, tb_ply), s);
                    assert_eq!(score_from_tt_with(s, ply, 100, tb_ply, true), s);
                }
            }
        }
    }
}

#[cfg(not(feature = "frozen"))]
struct Options(Vec<(&'static str, i32)>);

#[cfg(not(feature = "frozen"))]
impl Options {
    fn set(values: &[(&'static str, i32)]) -> Self {
        let saved = values.iter().map(|&(name, value)| {
            let id = match name {
                "FixMate50" => P::FixMate50, "FixTbPly" => P::FixTbPly,
                "FixR50Tt" => P::FixR50Tt, "SprContempt" => P::SprContempt,
                "SprMatScale" => P::SprMatScale, "SprEvalScale" => P::SprEvalScale,
                _ => panic!("unlisted test parameter"),
            };
            let old = p(id);
            assert!(crate::params::set(name, value));
            (name, old)
        }).collect();
        Self(saved)
    }
}

#[cfg(not(feature = "frozen"))]
impl Drop for Options {
    fn drop(&mut self) {
        for &(name, value) in &self.0 { crate::params::set(name, value); }
    }
}

#[cfg(not(feature = "frozen"))]
fn setup(fen: &str, ply: usize) -> (Board, Box<Searcher>) {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(crate::attacks::init);
    let b = Board::from_fen(fen).unwrap();
    let mut s = Box::new(Searcher::new(Arc::new(TT::new(1)), Arc::new(AtomicBool::new(false))));
    s.print = false;
    s.hashes.push(b.hash);
    if nnue::active() { nnue::refresh(&b, &mut s.acc[ply]); }
    (b, s)
}

#[cfg(not(feature = "frozen"))]
#[test]
fn mate_at_halfmove_99_and_100() {
    for fix in [0, 1] {
        // Exercise the fix independently of either TT fix and with/without
        // the legacy qsearch contempt draw detection.
        let _options = Options::set(&[("FixMate50", fix), ("FixTbPly", 0), ("FixR50Tt", 0)]);
        for contempt in [0, 25] {
            let _contempt = Options::set(&[("SprContempt", contempt)]);
            for clock in [98, 99] {
                let (b, mut s) = setup(&format!("7k/8/5KQ1/8/8/8/8/8 w - - {clock} 1"), 0);
                let mut child = b;
                child.make_move(b.parse_move("g6g7").unwrap());
                assert_eq!(child.halfmove, clock + 1);
                assert!(child.in_check() && child.legal_moves().is_empty());
                let expected = if clock == 99 && fix == 0 { 0 } else { -MATE + 1 };
                assert_eq!(s.negamax(&child, 1, -INF, INF, 1, false), expected);
                let expected_q = if clock == 99 && fix == 0 && contempt > 0 { 0 } else { -MATE + 1 };
                assert_eq!(s.qsearch(&child, -INF, INF, 1), expected_q);
            }
        }
    }
}

#[cfg(not(feature = "frozen"))]
#[test]
fn tt_fixes_reserve_eval_band_independently() {
    let (b, mut s) = setup("7k/8/8/8/8/8/8/1Q5K w - - 0 1", 1);
    for tb in [0, 1] {
        for r50 in [0, 1] {
            let _options = Options::set(&[("FixTbPly", tb), ("FixR50Tt", r50), ("SprMatScale", 0), ("SprEvalScale", 100)]);
            let limit = if tb > 0 || r50 > 0 { TB_BOUND - 1 } else { MATE_BOUND - 1 };
            assert_eq!(eval_limit(), limit);
            for sign in [-1, 1] {
                assert_eq!(s.adjusted_eval(&b, sign * 100_000), sign * limit);
                let keys = s.corr_keys(&b, 1);
                s.corr[b.stm][keys.pawn] = sign * CORR_MAX;
                assert_eq!(s.corrected(&b, sign * 100_000, &keys), sign * limit);
            }
        }
    }
}

#[cfg(not(feature = "frozen"))]
#[test]
fn main_and_qsearch_tt_reads_apply_the_current_clock() {
    for tb_ply in [0, 1] {
        let _options = Options::set(&[("FixMate50", 0), ("FixTbPly", tb_ply), ("FixR50Tt", 1)]);
        let (b, mut s) = setup("7k/8/8/8/8/8/8/1Q5K w - - 99 1", 7);
        for sign in [-1, 1] {
            for base in [MATE, TB_WIN] {
                for distance in [0, 1, 2] {
                    let score = sign * (base - 7 - distance);
                    let stored = score_to_tt(score, 7);
                    let expected = if distance > 1 { sign * (TB_BOUND - 1) } else { score };
                    s.tt.store(b.hash, Move::NULL, stored, NO_EVAL, 5, BOUND_EXACT, false);
                    assert_eq!(s.negamax(&b, 1, -1, 0, 7, false), expected);
                    assert_eq!(s.qsearch(&b, -1, 0, 7), expected);
                    s.tt.clear();
                }
            }
        }
    }
}

// Optional real-table integration reproduction. Explicitly enabled by the local
// validation command; no table downloads and no dependency on tables in CI.
#[cfg(not(feature = "frozen"))]
#[test]
#[ignore = "requires the existing local Syzygy tables and x25-big7 net/options"]
fn rc_bughunt_tb_reproduction() {
    let lab = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("testing");
    nnue::load_file(lab.join("engines/x25-big7.nnue").to_str().unwrap()).unwrap();
    let options = std::fs::read_to_string(lab.join("engines/x25-big7.options")).unwrap();
    for line in options.lines() {
        let (name, value) = line.split_once('=').unwrap();
        assert!(crate::params::set(name, value.parse().unwrap()));
    }
    let (_, pieces) = crate::tb::init(lab.join("syzygy").to_str().unwrap());
    assert!(pieces >= 3);
    for fix in [0, 1] {
        let _options = Options::set(&[("FixMate50", fix), ("FixTbPly", fix), ("FixR50Tt", fix)]);
        for (fen, sign) in [("7k/8/8/8/8/8/8/1Q5K w - - 0 1", 1), ("7k/8/8/8/8/8/8/1Q5K b - - 0 1", -1)] {
            let (b, mut s) = setup(fen, 3);
            let stored = s.negamax(&b, 2, -INF, INF, 3, false);
            let entry = s.tt.probe(b.hash).unwrap();
            if nnue::active() { nnue::refresh(&b, &mut s.acc[7]); }
            let warm = s.negamax(&b, 2, -1, 0, 7, false);
            s.tt.clear();
            let cold = s.negamax(&b, 2, -1, 0, 7, false);
            assert_eq!(stored, sign * (TB_WIN - 3));
            assert_eq!(cold, sign * (TB_WIN - 7));
            assert_eq!(warm, if fix > 0 { cold } else { stored });
            assert_eq!(entry.score, if fix > 0 { sign * TB_WIN } else { stored });
            println!("TB_PLY all_on={fix} sign={sign} ply3={stored} tt={} warm7={warm} cold7={cold}", entry.score);
            for threads in [1, 4] {
                let tt = Arc::new(TT::new(1));
                tt.store(b.hash, entry.mv, entry.score, entry.eval, entry.depth, entry.bound, entry.pv);
                std::thread::scope(|scope| {
                    let mut workers = Vec::new();
                    for _ in 0..threads {
                        let tt = tt.clone();
                        workers.push(scope.spawn(move || {
                            let (_, mut worker) = setup(fen, 7);
                            worker.tt = tt;
                            let warm = worker.negamax(&b, 2, -1, 0, 7, false);
                            let (_, mut cold_worker) = setup(fen, 7);
                            let cold = cold_worker.negamax(&b, 2, -1, 0, 7, false);
                            assert_eq!(cold, sign * (TB_WIN - 7));
                            assert_eq!(warm, if fix > 0 { cold } else { stored });
                        }));
                    }
                    for worker in workers { worker.join().unwrap(); }
                });
                println!("TB_PLY shared_tt threads={threads} all_on={fix} sign={sign}: warm/cold assertions passed");
            }
        }
    }
    // The cheap guard cannot solve the deliberately deferred clock-key issue:
    // a WDL probe's distance to a TB result is zero, not its distance to mate
    // or the next zeroing move. Reusing it at clock 99 remains optimistic.
    let _options = Options::set(&[("FixMate50", 1), ("FixTbPly", 1), ("FixR50Tt", 1)]);
    let (early, mut s) = setup("7k/8/5K1Q/8/8/8/8/8 b - - 0 1", 1);
    let first = s.negamax(&early, 2, -INF, INF, 1, false);
    let mut late = early;
    late.halfmove = 99;
    let warm = s.negamax(&late, 2, -1, 0, 1, false);
    s.tt.clear();
    let cold = s.negamax(&late, 2, -1, 0, 1, false);
    assert_eq!((first, warm, cold), (-TB_WIN + 1, -TB_WIN + 1, 0));
    println!("DEFERRED_TB_CLOCK all_on=1 early={first} warm99={warm} cold99={cold}; WDL distance is not DTZ");
    crate::tb::init("");
}
