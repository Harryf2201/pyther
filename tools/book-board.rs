//! Dependency-free board adapter; compile with rustc (no downloads).
#![allow(dead_code, static_mut_refs)]
#[path = "../src/types.rs"] mod types;
#[path = "../src/attacks.rs"] mod attacks;
#[path = "../src/board.rs"] mod board;
use board::Board;
use std::io::{self, BufRead, Write};

fn fields(b: &Board) -> String {
    let legal = b.legal_moves();
    let mut fen: Vec<String> = b.to_fen().split_whitespace().map(str::to_string).collect();
    // An uncapturable en-passant target does not distinguish chess positions.
    if !legal.iter().any(|m| m.is_ep()) { fen[3] = "-".into(); }
    let values = [100, 300, 300, 500, 900];
    let material: i32 = (0..5).map(|p| values[p] *
        (b.bb(0,p).count_ones() as i32 - b.bb(1,p).count_ones() as i32)).sum();
    format!("{}\t{}\t{}\t{}\t{}", fen.join(" "), b.in_check(), material,
        b.occ().count_ones(), legal.iter().map(|m| m.to_uci()).collect::<Vec<_>>().join(" "))
}
fn main() {
    attacks::init();
    let a: Vec<String> = std::env::args().collect();
    if a.get(1).map(String::as_str) == Some("pgn") {
        let text = std::fs::read_to_string(&a[2]).unwrap();
        let mut moves = String::new();
        let mut index = 0;
        let flush = |moves: &mut String, index: &mut usize| {
            if moves.trim().is_empty() { return; }
            *index += 1;
            let mut b = Board::startpos();
            let mut ucis = Vec::new();
            let mut clean = String::new();
            let mut nesting = 0;
            for c in moves.chars() {
                match c { '{'|'(' => nesting += 1, '}'|')' => nesting -= 1,
                    _ if nesting == 0 => clean.push(c), _ => {} }
            }
            for token in clean.split_whitespace() {
                if ["1-0","0-1","1/2-1/2","*"].contains(&token) { continue; }
                let t = token.trim_start_matches(|c:char| c.is_ascii_digit()).trim_start_matches('.')
                    .trim_end_matches(|c| c=='!' || c=='?');
                if t.is_empty() || t.starts_with('$') { continue; }
                let m = b.parse_san(t).unwrap_or_else(|| panic!("unparsed source {} token {}", index, t));
                ucis.push(m.to_uci()); b.make_move(m);
            }
            println!("{}\t{}\t{}", index, ucis.join(" "), fields(&b));
            moves.clear();
        };
        for line in text.lines() {
            if line.starts_with('[') { flush(&mut moves, &mut index); }
            else if !line.trim().is_empty() { moves.push(' '); moves.push_str(line); }
        }
        flush(&mut moves, &mut index);
        eprintln!("parsed {index} source openings");
        return;
    }
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let mut parts = line.split('\t');
        let mut b = Board::from_fen(parts.next().unwrap()).unwrap();
        for uci in parts.next().unwrap_or("").split_whitespace() {
            let m = b.parse_move(uci).expect("legal UCI extension"); b.make_move(m);
        }
        println!("{}", fields(&b)); io::stdout().flush().unwrap();
    }
}
