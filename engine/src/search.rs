//! Negamax alpha-beta search with iterative deepening, a transposition table,
//! quiescence search, and move ordering (TT move, MVV-LVA, killers, history).

use crate::eval::*;
use crate::movegen::generate_legal;
use crate::position::*;
use crate::types::*;
use std::time::Instant;

const MAX_PLY: usize = 128;

const BOUND_EXACT: u8 = 0;
const BOUND_LOWER: u8 = 1; // fail-high: score is a lower bound
const BOUND_UPPER: u8 = 2; // fail-low: score is an upper bound

#[derive(Clone, Copy)]
struct TTEntry {
    key: u64,
    mv: Move,
    score: i32,
    depth: i16,
    bound: u8,
}

impl TTEntry {
    const EMPTY: TTEntry = TTEntry {
        key: 0,
        mv: Move::NONE,
        score: 0,
        depth: -1,
        bound: BOUND_EXACT,
    };
}

#[derive(Clone)]
pub struct SearchLimits {
    pub depth: Option<u32>,
    pub movetime_ms: Option<u64>,
    pub nodes: Option<u64>,
}

impl Default for SearchLimits {
    fn default() -> Self {
        SearchLimits {
            depth: Some(64),
            movetime_ms: None,
            nodes: None,
        }
    }
}

pub struct SearchResult {
    pub best_move: Move,
    pub score: i32,
    pub depth: u32,
    pub pv: Vec<Move>,
    pub nodes: u64,
}

pub struct Search {
    tt: Vec<TTEntry>,
    tt_mask: usize,
    killers: [[Move; 2]; MAX_PLY],
    history: [[i32; 64]; 64],
    nodes: u64,
    start: Instant,
    limits: SearchLimits,
    stop: bool,
    /// Hashes of positions along the path + game history (for repetition draws).
    rep: Vec<u64>,
    /// If non-empty, only these moves are considered at the root (UCI searchmoves).
    root_moves: Vec<Move>,
    // Triangular PV table.
    pv: Vec<[Move; MAX_PLY]>,
    pv_len: [usize; MAX_PLY],
    /// Whether to print UCI `info` lines during iterative deepening.
    pub verbose: bool,
}

impl Search {
    /// Create a search with a transposition table of roughly `mb` megabytes.
    pub fn new(mb: usize) -> Search {
        let bytes = mb.max(1) * 1024 * 1024;
        let n = (bytes / std::mem::size_of::<TTEntry>()).next_power_of_two();
        Search {
            tt: vec![TTEntry::EMPTY; n],
            tt_mask: n - 1,
            killers: [[Move::NONE; 2]; MAX_PLY],
            history: [[0; 64]; 64],
            nodes: 0,
            start: Instant::now(),
            limits: SearchLimits::default(),
            stop: false,
            rep: Vec::with_capacity(512),
            root_moves: Vec::new(),
            pv: vec![[Move::NONE; MAX_PLY]; MAX_PLY],
            pv_len: [0; MAX_PLY],
            verbose: true,
        }
    }

    pub fn clear_tt(&mut self) {
        for e in self.tt.iter_mut() {
            *e = TTEntry::EMPTY;
        }
    }

    /// Run an iterative-deepening search. `history` is the list of position
    /// hashes that occurred earlier in the game (for repetition detection).
    pub fn go(&mut self, pos: &mut Position, limits: SearchLimits, history: &[u64]) -> SearchResult {
        self.go_with_roots(pos, limits, history, &[])
    }

    /// Like `go`, but if `root_moves` is non-empty the search only considers
    /// those moves at the root (used to find the best move among a subset, e.g.
    /// the second-best move when the top move is excluded).
    pub fn go_with_roots(
        &mut self,
        pos: &mut Position,
        limits: SearchLimits,
        history: &[u64],
        root_moves: &[Move],
    ) -> SearchResult {
        self.limits = limits;
        self.nodes = 0;
        self.stop = false;
        self.start = Instant::now();
        self.killers = [[Move::NONE; 2]; MAX_PLY];
        self.rep.clear();
        self.rep.extend_from_slice(history);
        self.root_moves = root_moves.to_vec();

        let max_depth = self.limits.depth.unwrap_or(64).min(MAX_PLY as u32 - 1);
        let mut best = SearchResult {
            best_move: Move::NONE,
            score: 0,
            depth: 0,
            pv: Vec::new(),
            nodes: 0,
        };

        for depth in 1..=max_depth {
            let score = self.negamax(pos, depth as i32, 0, -INF, INF);
            if self.stop && depth > 1 {
                break; // discard partial, unreliable iteration
            }
            let pv = self.collect_pv();
            best = SearchResult {
                best_move: pv.first().copied().unwrap_or(Move::NONE),
                score,
                depth,
                pv: pv.clone(),
                nodes: self.nodes,
            };
            if self.verbose {
                self.print_info(depth, score, &pv);
            }
            // Stop early on a forced mate or if out of time/nodes.
            if score.abs() >= MATE_VALUE - MAX_PLY as i32 {
                break;
            }
            if self.should_stop() {
                break;
            }
        }
        best
    }

    fn print_info(&self, depth: u32, score: i32, pv: &[Move]) {
        let elapsed = self.start.elapsed().as_millis().max(1) as u64;
        let nps = self.nodes * 1000 / elapsed;
        let score_str = if score.abs() >= MATE_VALUE - MAX_PLY as i32 {
            let mate_in = (MATE_VALUE - score.abs() + 1) / 2;
            format!("mate {}", if score > 0 { mate_in } else { -mate_in })
        } else {
            format!("cp {}", score)
        };
        let pv_str: Vec<String> = pv.iter().map(|m| m.to_uci()).collect();
        println!(
            "info depth {} score {} nodes {} nps {} time {} pv {}",
            depth,
            score_str,
            self.nodes,
            nps,
            elapsed,
            pv_str.join(" ")
        );
    }

    #[inline]
    fn should_stop(&self) -> bool {
        if let Some(mt) = self.limits.movetime_ms {
            if self.start.elapsed().as_millis() as u64 >= mt {
                return true;
            }
        }
        if let Some(n) = self.limits.nodes {
            if self.nodes >= n {
                return true;
            }
        }
        false
    }

    #[inline]
    fn check_time(&mut self) {
        if self.nodes & 2047 == 0 && self.should_stop() {
            self.stop = true;
        }
    }

    fn is_repetition(&self, hash: u64) -> bool {
        self.rep.iter().rev().any(|&k| k == hash)
    }

    fn negamax(&mut self, pos: &mut Position, depth: i32, ply: usize, mut alpha: i32, beta: i32) -> i32 {
        self.pv_len[ply] = 0;
        self.check_time();
        if self.stop {
            return 0;
        }

        let in_check = pos.in_check(pos.side);
        // Check-extension: don't drop into qsearch while in check.
        let depth = if in_check { depth + 1 } else { depth };

        if depth <= 0 {
            return self.quiescence(pos, ply, alpha, beta);
        }

        self.nodes += 1;

        // Draw detection (not at root).
        if ply > 0 {
            if pos.halfmove >= 100 || self.is_repetition(pos.hash) {
                return 0;
            }
        }

        let is_pv = beta - alpha > 1;
        let orig_alpha = alpha;

        // Transposition table probe.
        let tt_idx = (pos.hash as usize) & self.tt_mask;
        let entry = self.tt[tt_idx];
        let mut tt_move = Move::NONE;
        if entry.key == pos.hash {
            tt_move = entry.mv;
            if !is_pv && entry.depth as i32 >= depth {
                let s = self.adjust_mate_from_tt(entry.score, ply);
                match entry.bound {
                    BOUND_EXACT => return s,
                    BOUND_LOWER if s >= beta => return s,
                    BOUND_UPPER if s <= alpha => return s,
                    _ => {}
                }
            }
        }

        let mut moves = generate_legal(pos);
        if moves.is_empty() {
            // Checkmate or stalemate.
            return if in_check { -MATE_VALUE + ply as i32 } else { 0 };
        }

        // Restrict the root to the requested moves (UCI searchmoves), if any.
        if ply == 0 && !self.root_moves.is_empty() {
            let filtered: Vec<Move> = moves
                .iter()
                .copied()
                .filter(|m| self.root_moves.contains(m))
                .collect();
            if !filtered.is_empty() {
                moves = filtered;
            }
        }

        self.order_moves(pos, &mut moves, tt_move, ply);

        let mut best_score = -INF;
        let mut best_move = Move::NONE;

        for (i, &mv) in moves.iter().enumerate() {
            let undo = pos.make_move(mv);
            self.rep.push(undo.hash);

            let mut score;
            if i == 0 {
                score = -self.negamax(pos, depth - 1, ply + 1, -beta, -alpha);
            } else {
                // Late move reductions for quiet, non-check moves.
                let mut reduction = 0;
                if depth >= 3 && i >= 4 && !mv.is_capture() && !mv.is_promotion() && !in_check {
                    reduction = 1 + (i >= 8) as i32;
                }
                score = -self.negamax(pos, depth - 1 - reduction, ply + 1, -alpha - 1, -alpha);
                if score > alpha && reduction > 0 {
                    score = -self.negamax(pos, depth - 1, ply + 1, -alpha - 1, -alpha);
                }
                if score > alpha && score < beta {
                    score = -self.negamax(pos, depth - 1, ply + 1, -beta, -alpha);
                }
            }

            self.rep.pop();
            pos.unmake_move(mv, undo);

            if self.stop {
                return 0;
            }

            if score > best_score {
                best_score = score;
                best_move = mv;
                if score > alpha {
                    alpha = score;
                    self.update_pv(ply, mv);
                }
            }
            if alpha >= beta {
                // Beta cutoff: record killer/history for quiet moves.
                if !mv.is_capture() && !mv.is_promotion() {
                    self.store_killer(ply, mv);
                    self.history[mv.from() as usize][mv.to() as usize] += depth * depth;
                }
                break;
            }
        }

        // Store in TT.
        let bound = if best_score >= beta {
            BOUND_LOWER
        } else if best_score > orig_alpha {
            BOUND_EXACT
        } else {
            BOUND_UPPER
        };
        self.tt[tt_idx] = TTEntry {
            key: pos.hash,
            mv: best_move,
            score: self.adjust_mate_to_tt(best_score, ply),
            depth: depth as i16,
            bound,
        };

        best_score
    }

    fn quiescence(&mut self, pos: &mut Position, ply: usize, mut alpha: i32, beta: i32) -> i32 {
        self.check_time();
        if self.stop {
            return 0;
        }
        self.nodes += 1;

        let stand_pat = evaluate(pos);
        if stand_pat >= beta {
            return beta;
        }
        if stand_pat > alpha {
            alpha = stand_pat;
        }
        if ply >= MAX_PLY - 1 {
            return stand_pat;
        }

        // Only search captures and promotions.
        let mut moves: Vec<Move> = generate_legal(pos)
            .into_iter()
            .filter(|m| m.is_capture() || m.is_promotion())
            .collect();
        self.order_moves(pos, &mut moves, Move::NONE, ply);

        for mv in moves {
            let undo = pos.make_move(mv);
            let score = -self.quiescence(pos, ply + 1, -beta, -alpha);
            pos.unmake_move(mv, undo);
            if self.stop {
                return 0;
            }
            if score >= beta {
                return beta;
            }
            if score > alpha {
                alpha = score;
            }
        }
        alpha
    }

    // ---- move ordering ----
    fn order_moves(&self, pos: &Position, moves: &mut [Move], tt_move: Move, ply: usize) {
        const PIECE_VAL: [i32; 6] = [100, 320, 330, 500, 900, 20000];
        let mut scored: Vec<(i32, Move)> = moves
            .iter()
            .map(|&mv| {
                let s = if mv == tt_move {
                    2_000_000
                } else if mv.is_capture() || mv.is_promotion() {
                    let victim = if mv.is_ep() {
                        PieceType::Pawn
                    } else if let Some((_, pt)) = pos.mailbox[mv.to() as usize] {
                        pt
                    } else {
                        PieceType::Pawn
                    };
                    let attacker = pos.mailbox[mv.from() as usize].map(|(_, pt)| pt).unwrap_or(PieceType::Pawn);
                    let promo_bonus = mv.promo_piece().map(|p| PIECE_VAL[p.index()]).unwrap_or(0);
                    1_000_000 + PIECE_VAL[victim.index()] * 16 - PIECE_VAL[attacker.index()] + promo_bonus
                } else if mv == self.killers[ply][0] {
                    900_000
                } else if mv == self.killers[ply][1] {
                    800_000
                } else {
                    self.history[mv.from() as usize][mv.to() as usize]
                };
                (s, mv)
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        for (i, (_, mv)) in scored.into_iter().enumerate() {
            moves[i] = mv;
        }
    }

    fn store_killer(&mut self, ply: usize, mv: Move) {
        if self.killers[ply][0] != mv {
            self.killers[ply][1] = self.killers[ply][0];
            self.killers[ply][0] = mv;
        }
    }

    // ---- PV table ----
    fn update_pv(&mut self, ply: usize, mv: Move) {
        self.pv[ply][0] = mv;
        let child_len = self.pv_len[ply + 1];
        for i in 0..child_len {
            self.pv[ply][i + 1] = self.pv[ply + 1][i];
        }
        self.pv_len[ply] = child_len + 1;
    }

    fn collect_pv(&self) -> Vec<Move> {
        (0..self.pv_len[0]).map(|i| self.pv[0][i]).collect()
    }

    // Mate scores are stored relative to the node; adjust on store/probe.
    #[inline]
    fn adjust_mate_to_tt(&self, score: i32, ply: usize) -> i32 {
        if score >= MATE_VALUE - MAX_PLY as i32 {
            score + ply as i32
        } else if score <= -(MATE_VALUE - MAX_PLY as i32) {
            score - ply as i32
        } else {
            score
        }
    }
    #[inline]
    fn adjust_mate_from_tt(&self, score: i32, ply: usize) -> i32 {
        if score >= MATE_VALUE - MAX_PLY as i32 {
            score - ply as i32
        } else if score <= -(MATE_VALUE - MAX_PLY as i32) {
            score + ply as i32
        } else {
            score
        }
    }
}
