//! Universal Chess Interface (UCI) protocol loop.

use crate::movegen::generate_legal;
use crate::position::Position;
use crate::search::{Search, SearchLimits};
use crate::types::*;
use std::io::{self, BufRead, Write};

pub struct Engine {
    pos: Position,
    /// Hashes of positions played before the current one (for repetition).
    history: Vec<u64>,
    search: Search,
    tt_mb: usize,
}

impl Engine {
    pub fn new() -> Engine {
        let tt_mb = 64;
        Engine {
            pos: Position::startpos(),
            history: Vec::new(),
            search: Search::new(tt_mb),
            tt_mb,
        }
    }

    pub fn run(&mut self) {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let tokens: Vec<&str> = line.split_whitespace().collect();
            match tokens[0] {
                "uci" => self.cmd_uci(),
                "isready" => println!("readyok"),
                "ucinewgame" => self.search.clear_tt(),
                "position" => self.cmd_position(&tokens),
                "go" => self.cmd_go(&tokens),
                "setoption" => self.cmd_setoption(&tokens),
                "d" => println!("{}", self.pos.to_fen()),
                "quit" => break,
                _ => {}
            }
            io::stdout().flush().ok();
        }
    }

    fn cmd_uci(&self) {
        println!("id name BrilliantEngine 0.1");
        println!("id author chess-brilliant project");
        println!("option name Hash type spin default 64 min 1 max 4096");
        println!("uciok");
    }

    fn cmd_setoption(&mut self, tokens: &[&str]) {
        // setoption name Hash value 128
        if tokens.len() >= 5 && tokens[1] == "name" && tokens[2] == "Hash" && tokens[3] == "value" {
            if let Ok(mb) = tokens[4].parse::<usize>() {
                self.tt_mb = mb;
                self.search = Search::new(mb);
            }
        }
    }

    fn cmd_position(&mut self, tokens: &[&str]) {
        let mut idx = 1;
        if idx >= tokens.len() {
            return;
        }
        if tokens[idx] == "startpos" {
            self.pos = Position::startpos();
            idx += 1;
        } else if tokens[idx] == "fen" {
            // FEN spans the next up-to-6 tokens until "moves".
            let mut fen_parts = Vec::new();
            idx += 1;
            while idx < tokens.len() && tokens[idx] != "moves" {
                fen_parts.push(tokens[idx]);
                idx += 1;
            }
            match Position::from_fen(&fen_parts.join(" ")) {
                Ok(p) => self.pos = p,
                Err(e) => {
                    eprintln!("bad fen: {}", e);
                    return;
                }
            }
        }

        self.history.clear();
        if idx < tokens.len() && tokens[idx] == "moves" {
            idx += 1;
            for mv_str in &tokens[idx..] {
                if let Some(mv) = self.parse_move(mv_str) {
                    self.history.push(self.pos.hash);
                    self.pos.make_move(mv);
                } else {
                    eprintln!("illegal move in position: {}", mv_str);
                    break;
                }
            }
        }
    }

    /// Match a UCI move string (e.g. "e2e4", "e7e8q") against the legal moves.
    fn parse_move(&mut self, s: &str) -> Option<Move> {
        let from = string_to_square(&s[0..2])?;
        let to = string_to_square(&s[2..4])?;
        let promo = s.as_bytes().get(4).map(|&b| (b as char).to_ascii_lowercase());
        for mv in generate_legal(&mut self.pos) {
            if mv.from() == from && mv.to() == to {
                match (promo, mv.promo_piece()) {
                    (None, None) => return Some(mv),
                    (Some(c), Some(pt)) if pt.to_char().to_ascii_lowercase() == c => {
                        return Some(mv)
                    }
                    _ => continue,
                }
            }
        }
        None
    }

    fn cmd_go(&mut self, tokens: &[&str]) {
        let mut limits = SearchLimits {
            depth: None,
            movetime_ms: None,
            nodes: None,
        };
        let (mut wtime, mut btime, mut winc, mut binc, mut movestogo) =
            (None, None, None, None, None);
        let mut root_moves: Vec<Move> = Vec::new();

        let mut i = 1;
        while i < tokens.len() {
            let read_num = |i: usize| tokens.get(i + 1).and_then(|s| s.parse::<u64>().ok());
            match tokens[i] {
                "depth" => {
                    limits.depth = read_num(i).map(|n| n as u32);
                    i += 2;
                }
                "movetime" => {
                    limits.movetime_ms = read_num(i);
                    i += 2;
                }
                "nodes" => {
                    limits.nodes = read_num(i);
                    i += 2;
                }
                "wtime" => {
                    wtime = read_num(i);
                    i += 2;
                }
                "btime" => {
                    btime = read_num(i);
                    i += 2;
                }
                "winc" => {
                    winc = read_num(i);
                    i += 2;
                }
                "binc" => {
                    binc = read_num(i);
                    i += 2;
                }
                "movestogo" => {
                    movestogo = read_num(i);
                    i += 2;
                }
                "infinite" => {
                    limits.depth = Some(64);
                    i += 1;
                }
                "searchmoves" => {
                    // Consume the following move tokens as the root-move filter.
                    i += 1;
                    while i < tokens.len() {
                        match self.parse_move(tokens[i]) {
                            Some(mv) => {
                                root_moves.push(mv);
                                i += 1;
                            }
                            None => break,
                        }
                    }
                }
                _ => i += 1,
            }
        }

        // Derive a movetime from clock info if no explicit limit was given.
        if limits.movetime_ms.is_none() && limits.depth.is_none() && limits.nodes.is_none() {
            let (time, inc) = if self.pos.side == Color::White {
                (wtime, winc.unwrap_or(0))
            } else {
                (btime, binc.unwrap_or(0))
            };
            if let Some(t) = time {
                let moves = movestogo.unwrap_or(30).max(1);
                // Simple allocation: a slice of remaining time plus most of the increment.
                let budget = t / moves + inc * 3 / 4;
                limits.movetime_ms = Some(budget.min(t.saturating_sub(50)).max(10));
            } else {
                limits.depth = Some(64);
            }
        }

        let result = self
            .search
            .go_with_roots(&mut self.pos, limits, &self.history, &root_moves);
        println!("bestmove {}", result.best_move.to_uci());
        io::stdout().flush().ok();
    }
}
