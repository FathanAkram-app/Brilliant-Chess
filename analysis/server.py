"""Web server for a playable chess board backed by the analyzer.

Design for responsiveness:
  * Moves (move/undo/new) NEVER run the engine -- they update the board and
    return instantly, so the piece moves on screen with no delay.
  * Analysis is a SEPARATE endpoint that runs the engine on a snapshot of the
    position and reports which FEN it was computed for. The frontend draws the
    best-move arrow only when that FEN still matches the board, so stale results
    are discarded and a new move is never blocked by an in-flight search.
  * The server is multi-threaded with separate locks for the board (fast) and
    the engine (held only during a search), so moving while the engine thinks
    works fine.

Run:  python server.py     then open  http://127.0.0.1:5000
"""

from __future__ import annotations

import io
import os
import threading

import chess
import chess.pgn
from flask import Flask, jsonify, request, send_from_directory

from book import OpeningBook
from brilliancy import (
    BRILLIANT_MAX_LOSS,
    BRILLIANT_MIN_EVAL,
    BRILLIANT_MIN_SAC,
    _label_from_loss,
    judge_move,
)
from engine import MATE_SCORE, Engine
from learn import LearningStore
from see import PIECE_VALUE, best_opponent_capture

HERE = os.path.dirname(os.path.abspath(__file__))
WEB_DIR = os.path.join(HERE, "web")

app = Flask(__name__, static_folder=None)

board = chess.Board()
engine_ctx = Engine()
engine = engine_ctx.__enter__()
book = OpeningBook(os.path.join(HERE, "books", "book.bin"))
learned = LearningStore(os.path.join(HERE, "books", "learned.json"))

# When learning from a mistake, analyse deeper than live play so the stored
# knowledge is better than what a real-time search could produce.
LEARN_DEPTH = 20
LEARN_MOVETIME = 4.0

board_lock = threading.Lock()   # guards board reads/writes (fast)
engine_lock = threading.Lock()  # guards the single engine process (held during search)
cache_lock = threading.Lock()   # guards the analysis cache

DEFAULT_DEPTH = 12
ANALYSIS_TIME_CAP = 0.7  # seconds; bounds worst-case search so the UI stays snappy

# Cache engine analyses by position so revisiting (undo, transpositions) is instant.
analysis_cache: dict[str, dict] = {}


def fmt_eval(cp: int) -> str:
    if cp >= MATE_SCORE - 1000:
        return f"#{MATE_SCORE - cp}"
    if cp <= -(MATE_SCORE - 1000):
        return f"#-{MATE_SCORE + cp}"
    return f"{cp / 100:+.2f}"


def outcome_text(b: chess.Board) -> str:
    if b.is_checkmate():
        winner = "Black" if b.turn == chess.WHITE else "White"
        return f"Checkmate -- {winner} wins"
    if b.is_stalemate():
        return "Draw -- stalemate"
    if b.is_insufficient_material():
        return "Draw -- insufficient material"
    if b.is_fifty_moves():
        return "Draw -- fifty-move rule"
    if b.is_repetition():
        return "Draw -- threefold repetition"
    return "Game over"


def state_of(b: chess.Board) -> dict:
    """Fast board snapshot for rendering -- no engine involved."""
    last = b.move_stack[-1] if b.move_stack else None
    over = b.is_game_over()
    return {
        "fen": b.fen(),
        "turn": "white" if b.turn == chess.WHITE else "black",
        "legal_moves": [m.uci() for m in b.legal_moves],
        "last_move": [chess.square_name(last.from_square), chess.square_name(last.to_square)]
        if last
        else None,
        "in_check": b.is_check(),
        "check_square": chess.square_name(b.king(b.turn)) if b.is_check() else None,
        "move_number": b.fullmove_number,
        "game_over": over,
        "outcome": outcome_text(b) if over else None,
    }


def board_state() -> dict:
    with board_lock:
        return state_of(board)


def get_depth() -> int:
    try:
        return max(1, min(20, int(request.args.get("depth", DEFAULT_DEPTH))))
    except (TypeError, ValueError):
        return DEFAULT_DEPTH


@app.route("/")
def index():
    return send_from_directory(WEB_DIR, "index.html")


@app.route("/api/state")
def api_state():
    return jsonify(board_state())


@app.route("/api/move", methods=["POST"])
def api_move():
    data = request.get_json(force=True)
    uci = data.get("uci", "")
    with board_lock:
        try:
            move = chess.Move.from_uci(uci)
        except ValueError:
            return jsonify({"error": f"bad move {uci}"}), 400
        if move not in board.legal_moves:
            return jsonify({"error": f"illegal move {uci}"}), 400
        board.push(move)
    return jsonify(board_state())


@app.route("/api/undo", methods=["POST"])
def api_undo():
    with board_lock:
        if board.move_stack:
            board.pop()
    return jsonify(board_state())


@app.route("/api/new", methods=["POST"])
def api_new():
    with board_lock:
        board.reset()
    return jsonify(board_state())


@app.route("/api/engine_move", methods=["POST"])
def api_engine_move():
    """Search for the best move and play it. Uses the engine, so it takes time,
    but it does not block plain moves (separate lock + board snapshot)."""
    depth = get_depth()
    with board_lock:
        if board.is_game_over():
            return jsonify(board_state())
        snapshot = board.copy()
    # Prefer a book move, then a learned correction, then a live search.
    hit = book.probe(snapshot)
    le = learned.get(snapshot)
    if hit is not None:
        best = hit["best"]
    elif le is not None:
        best = chess.Move.from_uci(le["best_uci"])
    else:
        with engine_lock:
            best = engine.analyse(snapshot, depth=depth, movetime=ANALYSIS_TIME_CAP).best_move
    with board_lock:
        # Only play if the board hasn't changed under us.
        if board.fen() == snapshot.fen() and best in board.legal_moves:
            board.push(best)
    return jsonify(board_state())


def book_payload(snapshot: chess.Board, fen: str, hit: dict) -> dict:
    """Build an analysis response from an opening-book hit (no engine search)."""
    best = hit["best"]
    side = "white" if snapshot.turn == chess.WHITE else "black"
    # Show alternatives as "e4 45%  d4 44%  Nf3 9%".
    alts = "  ".join(f"{m['san']} {m['pct']}%" for m in hit["moves"])
    return {
        "fen": fen,
        "game_over": False,
        "source": "book",
        "side_to_move": side,
        "best_uci": best.uci(),
        "best_from": chess.square_name(best.from_square),
        "best_to": chess.square_name(best.to_square),
        "best_san": snapshot.san(best),
        "eval": "book",
        "white_cp": None,
        "depth": 0,
        "pv": [alts] if alts else [],
        "is_brilliant": False,
        "brilliant_text": "",
        "book_moves": hit["moves"],
    }


def learned_payload(b: chess.Board, fen: str, e: dict) -> dict:
    """Analysis response from a learned position (recalled, no live search)."""
    best = chess.Move.from_uci(e["best_uci"])
    side = "white" if b.turn == chess.WHITE else "black"
    mover_cp = e["white_cp"] if b.turn == chess.WHITE else -e["white_cp"]
    note = (f"Learned from your review: here you played {e.get('mistake_san','?')} "
            f"({e.get('label','a mistake')}). The best move is {e['best_san']}.")
    return {
        "fen": fen, "game_over": False, "source": "learned", "side_to_move": side,
        "best_uci": e["best_uci"],
        "best_from": chess.square_name(best.from_square),
        "best_to": chess.square_name(best.to_square),
        "best_san": e["best_san"],
        "eval": fmt_eval(mover_cp), "white_cp": e["white_cp"], "depth": e["depth"],
        "pv": [], "is_brilliant": False, "brilliant_text": "", "why": note,
    }


@app.route("/api/analyze")
def api_analyze():
    """Analyse the current position. Returns the FEN it was computed for so the
    frontend can discard the result if the user has already moved on.

    Order: opening book -> learned file -> cache -> engine search."""
    depth = get_depth()
    with board_lock:
        snapshot = board.copy()
    fen = snapshot.fen()

    if snapshot.is_game_over():
        return jsonify({"fen": fen, "game_over": True, "outcome": outcome_text(snapshot)})

    # 1. Opening book: instant, no search.
    hit = book.probe(snapshot)
    if hit is not None:
        return jsonify(book_payload(snapshot, fen, hit))

    # 1b. Learned file: a correction we computed from a past review.
    le = learned.get(snapshot)
    if le is not None:
        return jsonify(learned_payload(snapshot, fen, le))

    # 2. Cache (keyed by position + depth).
    key = f"{fen}|{depth}"
    with cache_lock:
        cached = analysis_cache.get(key)
    if cached is not None:
        return jsonify(cached)

    # 3. Engine search.
    with engine_lock:
        a = engine.analyse(snapshot, depth=depth, movetime=ANALYSIS_TIME_CAP)
        judgment = judge_move(
            engine, snapshot, a.best_move, depth=depth, pre=a, movetime=ANALYSIS_TIME_CAP
        )

    pv_san, b = [], snapshot.copy()
    for mv in a.pv:
        pv_san.append(b.san(mv))
        b.push(mv)

    white_cp = a.score_cp if snapshot.turn == chess.WHITE else -a.score_cp
    payload = {
        "fen": fen,
        "game_over": False,
        "source": "engine",
        "side_to_move": "white" if snapshot.turn == chess.WHITE else "black",
        "best_uci": a.best_move.uci(),
        "best_from": chess.square_name(a.best_move.from_square),
        "best_to": chess.square_name(a.best_move.to_square),
        "best_san": snapshot.san(a.best_move),
        "eval": fmt_eval(a.score_cp),
        "white_cp": white_cp,
        "depth": a.depth,
        "pv": pv_san,
        "is_brilliant": judgment.is_brilliant,
        "brilliant_text": judgment.explanation if judgment.is_brilliant else "",
        "why": explain_best(snapshot, a),
    }
    with cache_lock:
        if len(analysis_cache) > 5000:
            analysis_cache.clear()
        analysis_cache[key] = payload
    return jsonify(payload)


# ----------------------------------------------------------------------------
# Game review: load a PGN/FEN and step through it with per-move grading.
# ----------------------------------------------------------------------------

review: dict = {"loaded": False, "start_fen": chess.STARTING_FEN, "moves": [], "sans": [], "headers": {}}
analysis_obj_cache: dict[str, object] = {}   # fen|depth -> engine Analysis
judge_cache: dict[str, dict] = {}            # fen|move|depth -> judgment dict


def eval_obj(b: chess.Board, depth: int):
    """Engine analysis of a position, cached by FEN+depth (review uses no book
    so every position has a real evaluation)."""
    key = f"{b.fen()}|{depth}"
    with cache_lock:
        o = analysis_obj_cache.get(key)
    if o is not None:
        return o
    with engine_lock:
        o = engine.analyse(b, depth=depth, movetime=ANALYSIS_TIME_CAP)
    with cache_lock:
        if len(analysis_obj_cache) > 4000:
            analysis_obj_cache.clear()
        analysis_obj_cache[key] = o
    return o


def analysis_dict_from_obj(b: chess.Board, o) -> dict:
    pv_san, bb = [], b.copy()
    for mv in o.pv:
        pv_san.append(bb.san(mv))
        bb.push(mv)
    white_cp = o.score_cp if b.turn == chess.WHITE else -o.score_cp
    return {
        "fen": b.fen(),
        "game_over": False,
        "source": "engine",
        "side_to_move": "white" if b.turn == chess.WHITE else "black",
        "best_uci": o.best_move.uci(),
        "best_from": chess.square_name(o.best_move.from_square),
        "best_to": chess.square_name(o.best_move.to_square),
        "best_san": b.san(o.best_move),
        "eval": fmt_eval(o.score_cp),
        "white_cp": white_cp,
        "depth": o.depth,
        "pv": pv_san,
        "is_brilliant": False,
        "brilliant_text": "",
        "why": explain_best(b, o),
    }


GREAT_GAP = 150  # cp the next-best move must trail by for the played move to be "Great"
second_cache: dict[str, object] = {}  # fen|depth -> second-best score (mover POV) or None


def describe_move(board_prev: chess.Board, move: chess.Move, board_after: chess.Board) -> dict:
    """Concrete, observable features of a move (for explanations)."""
    cap = None
    if board_prev.is_capture(move):
        cap = "pawn" if board_prev.is_en_passant(move) else chess.piece_name(
            board_prev.piece_at(move.to_square).piece_type
        )
    return {
        "mate": board_after.is_checkmate(),
        "check": board_after.is_check(),
        "castle": board_prev.is_castling(move),
        "promo": chess.piece_name(move.promotion) if move.promotion else None,
        "capture": board_prev.is_capture(move),
        "captured": cap,
        "piece": chess.piece_name(board_prev.piece_at(move.from_square).piece_type),
    }


def second_best_cp(board_prev: chess.Board, best_move: chess.Move, depth: int):
    """Score of the best move EXCLUDING `best_move` (mover's POV), or None."""
    key = f"{board_prev.fen()}|{depth}"
    with cache_lock:
        if key in second_cache:
            return second_cache[key]
    others = [m for m in board_prev.legal_moves if m != best_move]
    val = None
    if others:
        with engine_lock:
            a2 = engine.analyse(
                board_prev, depth=depth, movetime=ANALYSIS_TIME_CAP, root_moves=others
            )
        val = a2.score_cp
    with cache_lock:
        second_cache[key] = val
    return val


def consequence(eval_after: int, board_after: chess.Board) -> str:
    """A short clause describing what a bad move allows the opponent."""
    if eval_after <= -(MATE_SCORE - 1000):
        return " It allows a forced mate."
    hang = best_opponent_capture(board_after)
    if hang >= 200:
        return f" It hangs about {hang / 100:.1f} points of material."
    return ""


def explain_played(label, f, eval_after, loss, best_san, sacrifice, gap, board_after) -> str:
    e = fmt_eval(eval_after)  # mover's POV
    if label == "Brilliant":
        if f["mate"]:
            outcome = "and forces checkmate"
        elif eval_after >= 300:
            outcome = f"and keeps a winning position ({e})"
        else:
            outcome = f"yet the position stays good ({e})"
        return (f"Brilliant — it sacrifices about {sacrifice / 100:.1f} points of material "
                f"{outcome}. The kind of sacrifice that's easy to overlook.")
    if label == "Great":
        base = (f"the only move that holds up here — every alternative was clearly worse "
                f"(by ~{gap / 100:.1f} points)")
        extra = " It also forces mate." if f["mate"] else (" It comes with check." if f["check"] else "")
        return f"Great move — {base}.{extra}"
    if label == "Best":
        if f["mate"]:
            why = "it delivers checkmate"
        elif f["check"] and f["capture"]:
            why = f"it gives check and wins a {f['captured']}"
        elif f["capture"]:
            why = f"it wins material, capturing a {f['captured']}"
        elif f["castle"]:
            why = "it castles the king to safety"
        elif f["promo"]:
            why = f"it promotes to a {f['promo']}"
        elif f["check"]:
            why = "it gives check"
        else:
            why = f"it keeps the best evaluation ({e})"
        return f"Best move — {why}. Nothing else is better."
    if label == "Excellent":
        return (f"Excellent — almost the top move ({best_san}); only {loss / 100:.2f} behind and "
                f"keeps your position ({e}).")
    if label == "Good":
        return f"A solid move. The engine slightly preferred {best_san} (by {loss / 100:.2f})."
    if label == "Inaccuracy":
        return f"Inaccuracy — {best_san} was better; your evaluation slipped to {e} (−{loss / 100:.2f})."
    if label == "Mistake":
        return f"Mistake — it loses about {loss / 100:.1f} points.{consequence(eval_after, board_after)} {best_san} was stronger."
    if label == "Blunder":
        return f"Blunder — a serious error ({e}).{consequence(eval_after, board_after)} {best_san} was needed."
    return ""


def explain_best(board: chess.Board, o) -> str:
    """Why the engine's suggested move is best (for the arrow)."""
    after = board.copy()
    after.push(o.best_move)
    f = describe_move(board, o.best_move, after)
    e = fmt_eval(o.score_cp)
    if f["mate"]:
        return "Forces checkmate."
    if f["check"] and f["capture"]:
        return f"Gives check and wins a {f['captured']} ({e})."
    if f["capture"]:
        return f"Wins material — captures a {f['captured']} ({e})."
    if f["castle"]:
        return f"Castles to safety ({e})."
    if f["promo"]:
        return f"Promotes to a {f['promo']} ({e})."
    if f["check"]:
        return f"Gives check ({e})."
    return f"Keeps the best evaluation ({e})."


def played_judgment(board_prev: chess.Board, move: chess.Move, depth: int) -> dict:
    """Grade the move played in board_prev, reusing cached position evals."""
    key = f"{board_prev.fen()}|{move.uci()}|{depth}"
    with cache_lock:
        cached = judge_cache.get(key)
    if cached is not None:
        return cached

    pre = eval_obj(board_prev, depth)
    board_after = board_prev.copy()
    board_after.push(move)
    o_after = eval_obj(board_after, depth)

    mover = board_prev.turn
    eval_before = pre.score_cp          # mover's POV (mover is side to move in board_prev)
    eval_after = -o_after.score_cp      # convert opponent POV back to mover POV
    played_best = move == pre.best_move
    # Playing the engine's own top move loses nothing by definition; the two
    # separate searches can disagree slightly, so pin the loss to 0 here.
    loss = 0 if played_best else max(0, eval_before - eval_after)

    captured_value = 0
    if board_prev.is_capture(move):
        if board_prev.is_en_passant(move):
            captured_value = PIECE_VALUE[chess.PAWN]
        else:
            captured_value = PIECE_VALUE[board_prev.piece_at(move.to_square).piece_type]
    sacrifice = best_opponent_capture(board_after) - captured_value

    is_brilliant = (
        loss <= BRILLIANT_MAX_LOSS
        and sacrifice >= BRILLIANT_MIN_SAC
        and eval_after >= BRILLIANT_MIN_EVAL
    )
    gap = None
    if is_brilliant:
        label = "Brilliant"
    else:
        label = _label_from_loss(loss)
        # "Great": the played move is the best AND the only good move (alternatives
        # clearly worse) in a position that isn't already trivially won.
        if played_best and label in ("Best", "Excellent") and -150 <= eval_before <= 600:
            sb = second_best_cp(board_prev, pre.best_move, depth)
            if sb is not None:
                gap = eval_before - sb
                if gap >= GREAT_GAP:
                    label = "Great"

    feats = describe_move(board_prev, move, board_after)
    explanation = explain_played(
        label, feats, eval_after, loss, board_prev.san(pre.best_move), sacrifice, gap or 0, board_after
    )

    result = {
        "san": board_prev.san(move),
        "uci": move.uci(),
        "label": label,
        "is_brilliant": is_brilliant,
        "loss": loss,
        "eval_before_white": eval_before if mover == chess.WHITE else -eval_before,
        "eval_after_white": eval_after if mover == chess.WHITE else -eval_after,
        "best_san": board_prev.san(pre.best_move),
        "best_uci": pre.best_move.uci(),
        "played_best": played_best,
        "mover": "white" if mover == chess.WHITE else "black",
        "explanation": explanation,
    }
    with cache_lock:
        if len(judge_cache) > 4000:
            judge_cache.clear()
        judge_cache[key] = result
    return result


def review_board(n: int) -> chess.Board:
    b = chess.Board(review["start_fen"])
    for i in range(min(n, len(review["moves"]))):
        b.push(chess.Move.from_uci(review["moves"][i]))
    return b


@app.route("/api/review/load", methods=["POST"])
def api_review_load():
    data = request.get_json(force=True)
    pgn = (data.get("pgn") or "").strip()
    fen = (data.get("fen") or "").strip()

    if pgn:
        game = chess.pgn.read_game(io.StringIO(pgn))
        if game is None:
            return jsonify({"error": "Could not parse PGN."}), 400
        b = game.board()
        start_fen = b.fen()
        moves, sans = [], []
        for mv in game.mainline_moves():
            sans.append(b.san(mv))
            moves.append(mv.uci())
            b.push(mv)
        headers = dict(game.headers)
    elif fen:
        try:
            b = chess.Board(fen)
        except ValueError:
            return jsonify({"error": "Invalid FEN."}), 400
        start_fen, moves, sans, headers = b.fen(), [], [], {}
    else:
        return jsonify({"error": "Provide a PGN or a FEN."}), 400

    review.update(loaded=True, start_fen=start_fen, moves=moves, sans=sans, headers=headers)
    return jsonify({
        "ok": True,
        "ply_count": len(moves),
        "start_fen": start_fen,
        "moves": [{"san": s, "uci": u} for s, u in zip(sans, moves)],
        "headers": {k: headers.get(k, "") for k in ("White", "Black", "Result", "Event", "Date")},
    })


@app.route("/api/review/from_play", methods=["POST"])
def api_review_from_play():
    """Load the game currently on the free-play board into review mode."""
    with board_lock:
        moves = [m.uci() for m in board.move_stack]
    if not moves:
        return jsonify({"error": "No moves played yet -- play some moves first."}), 400

    # The play board always starts from the standard position (New game resets it).
    b = chess.Board()
    sans = []
    for u in moves:
        mv = chess.Move.from_uci(u)
        sans.append(b.san(mv))
        b.push(mv)
    headers = {"White": "You", "Black": "You / Engine", "Event": "Web UI game"}
    review.update(loaded=True, start_fen=chess.STARTING_FEN, moves=moves, sans=sans, headers=headers)
    return jsonify({
        "ok": True,
        "ply_count": len(moves),
        "start_fen": chess.STARTING_FEN,
        "moves": [{"san": s, "uci": u} for s, u in zip(sans, moves)],
        "headers": {k: headers.get(k, "") for k in ("White", "Black", "Result", "Event", "Date")},
    })


@app.route("/api/review/ply")
def api_review_ply():
    if not review["loaded"]:
        return jsonify({"error": "No game loaded."}), 400
    depth = get_depth()
    count = len(review["moves"])
    try:
        n = max(0, min(count, int(request.args.get("n", 0))))
    except (TypeError, ValueError):
        n = 0

    b = review_board(n)
    payload = state_of(b)
    payload["ply"] = n
    payload["ply_count"] = count

    if b.is_game_over():
        payload["analysis"] = {"fen": b.fen(), "game_over": True, "outcome": outcome_text(b)}
    else:
        payload["analysis"] = analysis_dict_from_obj(b, eval_obj(b, depth))

    if n >= 1:
        move = chess.Move.from_uci(review["moves"][n - 1])
        payload["played"] = played_judgment(review_board(n - 1), move, depth)
    else:
        payload["played"] = None
    return jsonify(payload)


@app.route("/api/review/learn", methods=["POST"])
def api_review_learn():
    """Learn from every mistake in the reviewed game: for each Inaccuracy/
    Mistake/Blunder, analyse the position deeper and remember the right move."""
    if not review["loaded"]:
        return jsonify({"error": "No game loaded."}), 400
    depth = get_depth()
    bad = {"Inaccuracy", "Mistake", "Blunder"}
    new_count = 0
    examples = []
    for n in range(1, len(review["moves"]) + 1):
        bp = review_board(n - 1)
        move = chess.Move.from_uci(review["moves"][n - 1])
        j = played_judgment(bp, move, depth)
        if j["label"] not in bad:
            continue
        # Deeper search of the position BEFORE the mistake = the lesson.
        with engine_lock:
            a = engine.analyse(bp, depth=LEARN_DEPTH, movetime=LEARN_MOVETIME)
        white_cp = a.score_cp if bp.turn == chess.WHITE else -a.score_cp
        entry = {
            "best_uci": a.best_move.uci(),
            "best_san": bp.san(a.best_move),
            "white_cp": white_cp,
            "depth": a.depth,
            "mistake_san": j["san"],
            "label": j["label"],
        }
        if learned.put(bp, entry) and len(examples) < 8:
            examples.append({"move_no": (n + 1) // 2, "played": j["san"],
                             "label": j["label"], "best": entry["best_san"]})
        new_count += 1
    learned.save()
    return jsonify({"learned": new_count, "examples": examples, "total": len(learned)})


@app.route("/api/learn/stats")
def api_learn_stats():
    return jsonify({"total": len(learned)})


if __name__ == "__main__":
    print("Chess analyzer board running at http://127.0.0.1:5000")
    app.run(host="127.0.0.1", port=5000, threaded=True)
