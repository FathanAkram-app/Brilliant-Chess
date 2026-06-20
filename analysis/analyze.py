"""Command-line interface for the chess analyzer.

Examples:
  # Best move, evaluation, and predicted line for a position:
  python analyze.py fen "5r1k/6pp/7N/3Q4/8/8/6PP/6K1 w - - 0 1"

  # Judge a specific move (is it brilliant? a blunder?):
  python analyze.py move "<FEN>" d5g8

  # Is there a brilliant move available right now?
  python analyze.py brilliant "<FEN>"

  # Predict the whole game to its end from a position:
  python analyze.py predict "<FEN>"

  # Scan a PGN file and report every move's quality (highlighting brilliancies):
  python analyze.py pgn game.pgn
"""

from __future__ import annotations

import argparse
import sys

import chess
import chess.pgn

from brilliancy import find_brilliant_move, judge_move
from engine import Engine

# ANSI colours for the terminal.
C = {
    "Brilliant": "\033[1;36m",  # bright cyan
    "Best": "\033[1;32m",
    "Excellent": "\033[32m",
    "Good": "\033[0m",
    "Inaccuracy": "\033[33m",
    "Mistake": "\033[35m",
    "Blunder": "\033[1;31m",
}
RESET = "\033[0m"


def color(label: str) -> str:
    return C.get(label, "")


def fmt_eval(cp: int) -> str:
    from engine import MATE_SCORE

    if cp >= MATE_SCORE - 1000:
        return f"#{MATE_SCORE - cp}"
    if cp <= -(MATE_SCORE - 1000):
        return f"#-{MATE_SCORE + cp}"
    return f"{cp/100:+.2f}"


def cmd_fen(args, engine: Engine) -> None:
    board = chess.Board(args.fen)
    a = engine.analyse(board, depth=args.depth, label="Thinking...")
    print(f"Position: {board.fen()}")
    print(f"Side to move: {'White' if board.turn else 'Black'}")
    print(f"Best move: {board.san(a.best_move)}  ({a.best_move.uci()})")
    print(f"Evaluation: {fmt_eval(a.score_cp)}  (depth {a.depth})")
    # Render the predicted line in SAN.
    line, b = [], board.copy()
    for mv in a.pv:
        line.append(b.san(mv))
        b.push(mv)
    print(f"Predicted line: {' '.join(line)}")

    # Reuse the analysis we already have instead of re-searching the position.
    brill = judge_move(engine, board, a.best_move, depth=args.depth, pre=a)
    if brill.is_brilliant:
        print(f"\n{color('Brilliant')}!! Brilliant move available: "
              f"{brill.san}{RESET} -- {brill.explanation}")


def cmd_move(args, engine: Engine) -> None:
    board = chess.Board(args.fen)
    move = chess.Move.from_uci(args.move)
    if move not in board.legal_moves:
        print(f"Illegal move {args.move} in this position.", file=sys.stderr)
        sys.exit(1)
    san = board.san(move)
    print(f"Evaluating {san} ...", flush=True)
    j = judge_move(engine, board, move, depth=args.depth)
    mark = "!!" if j.is_brilliant else ""
    print(f"{color(j.label)}{j.san}{mark}  [{j.label}]{RESET}")
    print(f"  eval: {fmt_eval(j.eval_before)} -> {fmt_eval(j.eval_after)} "
          f"(lost {j.loss/100:.2f})")
    if j.sacrifice:
        print(f"  material sacrificed: ~{j.sacrifice/100:.1f}")
    print(f"  {j.explanation}")


def cmd_brilliant(args, engine: Engine) -> None:
    board = chess.Board(args.fen)
    brill = find_brilliant_move(engine, board, depth=args.depth)
    if brill:
        print(f"{color('Brilliant')}{brill.san}!!{RESET}  {brill.explanation}")
    else:
        a = engine.analyse(board, depth=args.depth)
        print(f"No brilliant move. Best is {board.san(a.best_move)} "
              f"({fmt_eval(a.score_cp)}).")


def cmd_predict(args, engine: Engine) -> None:
    board = chess.Board(args.fen)
    print("Predicting the game to its end (engine vs engine)...\n")
    moves = engine.rollout_to_end(board, depth=args.depth, max_plies=args.max_plies)
    b = board.copy()
    out = []
    for i, mv in enumerate(moves):
        san = b.san(mv)
        if b.turn == chess.WHITE:
            out.append(f"{b.fullmove_number}.{san}")
        else:
            out.append(san)
        b.push(mv)
    print(" ".join(out))
    print(f"\nResult: {b.result()}  ({_outcome(b)})")


def _outcome(board: chess.Board) -> str:
    if board.is_checkmate():
        return "checkmate"
    if board.is_stalemate():
        return "stalemate"
    if board.is_insufficient_material():
        return "insufficient material"
    if board.can_claim_threefold_repetition():
        return "threefold repetition"
    if board.is_fifty_moves():
        return "fifty-move rule"
    return "unfinished (ply limit reached)"


def cmd_pgn(args, engine: Engine) -> None:
    with open(args.file, encoding="utf-8") as fh:
        game = chess.pgn.read_game(fh)
    if game is None:
        print("No game found in PGN.", file=sys.stderr)
        sys.exit(1)
    board = game.board()
    moves = list(game.mainline_moves())
    total = len(moves)
    print(f"Scanning {total} moves at depth {args.depth} ...\n", flush=True)

    # Judge move-by-move so progress streams live instead of all at the end.
    b = board.copy()
    brilliancies = []
    for i, mv in enumerate(moves, 1):
        num = b.fullmove_number
        side = "" if b.turn == chess.WHITE else "..."
        prefix = f"{num}.{side}"
        san = b.san(mv)
        print(f"[{i}/{total}] {prefix:>7} {san} ...", end="", flush=True)
        j = judge_move(engine, b, mv, depth=args.depth)
        mark = "!!" if j.is_brilliant else ""
        print(f"\r[{i}/{total}] {prefix:>7} {color(j.label)}{j.san}{mark:<2} "
              f"[{j.label}] {fmt_eval(j.eval_after)}{RESET}        ", flush=True)
        if j.is_brilliant:
            brilliancies.append((prefix, j))
        b.push(j.move)

    print("\n=== Brilliant moves ===")
    if brilliancies:
        for prefix, j in brilliancies:
            print(f"  {prefix} {j.san}!! -- {j.explanation}")
    else:
        print("  none found")


def main() -> None:
    p = argparse.ArgumentParser(description="Chess analyzer: predict lines and find brilliant moves.")
    p.add_argument("--depth", type=int, default=14, help="search depth (default 14)")
    p.add_argument("-v", "--verbose", action="store_true",
                   help="stream the engine's per-depth search output (watch it think)")
    sub = p.add_subparsers(dest="cmd", required=True)

    sp = sub.add_parser("fen", help="analyse a position")
    sp.add_argument("fen")
    sp.set_defaults(func=cmd_fen)

    sp = sub.add_parser("move", help="judge a specific move")
    sp.add_argument("fen")
    sp.add_argument("move", help="move in UCI form, e.g. e2e4")
    sp.set_defaults(func=cmd_move)

    sp = sub.add_parser("brilliant", help="find a brilliant move if one exists")
    sp.add_argument("fen")
    sp.set_defaults(func=cmd_brilliant)

    sp = sub.add_parser("predict", help="predict the game to its end")
    sp.add_argument("fen")
    sp.add_argument("--max-plies", type=int, default=200)
    sp.set_defaults(func=cmd_predict)

    sp = sub.add_parser("pgn", help="scan a PGN game for move quality")
    sp.add_argument("file")
    sp.set_defaults(func=cmd_pgn)

    args = p.parse_args()
    # Line-buffer stdout so progress appears live even when piped or redirected.
    try:
        sys.stdout.reconfigure(line_buffering=True)
    except (AttributeError, ValueError):
        pass
    with Engine(verbose=args.verbose) as engine:
        args.func(args, engine)


if __name__ == "__main__":
    main()
