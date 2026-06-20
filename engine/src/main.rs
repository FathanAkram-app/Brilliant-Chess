mod eval;
mod movegen;
mod perft;
mod position;
mod search;
mod tables;
mod types;
mod uci;
mod zobrist;

use position::Position;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // perft harness: chess_engine perft <depth> [fen]
    if args.len() >= 3 && args[1] == "perft" {
        let depth: u32 = args[2].parse().expect("depth must be an integer");
        let mut pos = if args.len() >= 4 {
            Position::from_fen(&args[3..].join(" ")).expect("bad FEN")
        } else {
            Position::startpos()
        };
        let start = std::time::Instant::now();
        let nodes = perft::perft_divide(&mut pos, depth);
        let dt = start.elapsed().as_secs_f64();
        println!(
            "Time: {:.3}s  ({:.0} nodes/s)",
            dt,
            nodes as f64 / dt.max(1e-9)
        );
        return;
    }

    // hashcheck: validate incremental Zobrist hashing against full recompute.
    if args.len() >= 2 && args[1] == "hashcheck" {
        let mut pos = Position::startpos();
        let ok = hashcheck(&mut pos, 4);
        println!("hashcheck depth 4: {}", if ok { "OK" } else { "FAILED" });
        return;
    }

    // Default: run the UCI protocol loop.
    let mut engine = uci::Engine::new();
    engine.run();
}

/// Recursively verify that the incremental hash matches a full recompute.
fn hashcheck(pos: &mut Position, depth: u32) -> bool {
    if pos.hash != pos.compute_hash() {
        eprintln!("hash mismatch at fen {}", pos.to_fen());
        return false;
    }
    if depth == 0 {
        return true;
    }
    for mv in movegen::generate_legal(pos) {
        let undo = pos.make_move(mv);
        if !hashcheck(pos, depth - 1) {
            return false;
        }
        pos.unmake_move(mv, undo);
        if pos.hash != pos.compute_hash() {
            eprintln!("hash mismatch after unmake of {}", mv.to_uci());
            return false;
        }
    }
    true
}
