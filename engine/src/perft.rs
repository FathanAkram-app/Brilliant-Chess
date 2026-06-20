//! Perft: count leaf nodes of the move tree to a given depth.
//! Used to validate move generation against known reference counts.

use crate::movegen::generate_legal;
use crate::position::Position;

pub fn perft(pos: &mut Position, depth: u32) -> u64 {
    if depth == 0 {
        return 1;
    }
    let moves = generate_legal(pos);
    if depth == 1 {
        return moves.len() as u64;
    }
    let mut nodes = 0u64;
    for mv in moves {
        let undo = pos.make_move(mv);
        nodes += perft(pos, depth - 1);
        pos.unmake_move(mv, undo);
    }
    nodes
}

/// Perft with a per-root-move breakdown (matches Stockfish's `go perft` output).
pub fn perft_divide(pos: &mut Position, depth: u32) -> u64 {
    let moves = generate_legal(pos);
    let mut total = 0u64;
    for mv in moves {
        let undo = pos.make_move(mv);
        let n = if depth <= 1 { 1 } else { perft(pos, depth - 1) };
        pos.unmake_move(mv, undo);
        println!("{}: {}", mv.to_uci(), n);
        total += n;
    }
    println!("\nNodes: {}", total);
    total
}
