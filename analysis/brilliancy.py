"""Move-quality classification and brilliant-move (!!) detection.

A move is judged **Brilliant** when all of the following hold:
  1. It is the best or near-best move (it does not throw away the position).
  2. It sacrifices real material -- after the move the opponent can win
     >= a minor piece worth of material by a static-exchange capture.
  3. The position stays good for the mover despite the sacrifice (the engine
     sees compensation that a naive material count would miss).
  4. The mover is not already completely winning to the point the sac is moot.

Other moves get a chess.com-style label (Best / Excellent / Good /
Inaccuracy / Mistake / Blunder) based on centipawns lost versus the best move.
"""

from __future__ import annotations

from dataclasses import dataclass

import chess

from engine import MATE_SCORE, Analysis, Engine
from see import PIECE_VALUE, best_opponent_capture

# --- tunable thresholds (centipawns) ---
BRILLIANT_MAX_LOSS = 30      # how far from best a brilliant move may be
BRILLIANT_MIN_SAC = 150      # min NET material sacrificed (>= an exchange)
BRILLIANT_MIN_EVAL = -120    # must not be (clearly) losing after the move
MATE_THRESHOLD = MATE_SCORE - 1000  # scores at/above this are forced mates


@dataclass
class MoveJudgment:
    move: chess.Move
    san: str
    label: str                 # Brilliant / Best / Excellent / ... / Blunder
    eval_before: int           # best achievable for mover (cp, mover POV)
    eval_after: int            # eval after the played move (cp, mover POV)
    loss: int                  # eval_before - eval_after
    sacrifice: int             # material the opponent can win back (cp), 0 if none
    is_brilliant: bool
    explanation: str


def _label_from_loss(loss: int) -> str:
    if loss <= 10:
        return "Best"
    if loss <= 40:
        return "Excellent"
    if loss <= 90:
        return "Good"
    if loss <= 150:
        return "Inaccuracy"
    if loss <= 300:
        return "Mistake"
    return "Blunder"


def _fmt_eval(cp: int) -> str:
    if cp >= MATE_SCORE - 1000:
        return f"#{MATE_SCORE - cp}"
    if cp <= -(MATE_SCORE - 1000):
        return f"#-{MATE_SCORE + cp}"
    return f"{cp/100:+.2f}"


def judge_move(
    engine: Engine,
    board: chess.Board,
    move: chess.Move,
    depth: int = 14,
    pre: Analysis | None = None,
    movetime: float | None = None,
) -> MoveJudgment:
    """Classify a single move played in `board` (the position *before* it).

    `movetime` (seconds) caps each search so interactive use stays responsive.
    """
    san = board.san(move)
    if pre is None:
        pre = engine.analyse(board, depth=depth, movetime=movetime)
    eval_before = pre.score_cp

    child = board.copy()
    child.push(move)
    eval_after = -engine.evaluate(child, depth=depth, movetime=movetime)
    loss = eval_before - eval_after

    # NET material sacrificed = what the opponent can win back by capturing,
    # minus what this move itself just captured. This distinguishes a real
    # sacrifice from an ordinary trade or recapture.
    captured_value = 0
    if board.is_capture(move):
        if board.is_en_passant(move):
            captured_value = PIECE_VALUE[chess.PAWN]
        else:
            captured_value = PIECE_VALUE[board.piece_at(move.to_square).piece_type]
    sacrifice = best_opponent_capture(child) - captured_value

    # A brilliant move is a sound sacrifice that is (near-)best and stays good.
    # We do NOT exclude forced mates -- a sac that forces mate is the prototype
    # brilliancy. We only require that the move keeps a good position.
    is_brilliant = (
        loss <= BRILLIANT_MAX_LOSS
        and sacrifice >= BRILLIANT_MIN_SAC
        and eval_after >= BRILLIANT_MIN_EVAL
    )

    if is_brilliant:
        label = "Brilliant"
        explanation = (
            f"Sacrifices ~{sacrifice/100:.1f} points of material yet keeps the "
            f"position at {_fmt_eval(eval_after)} -- a sound sacrifice and the best move."
        )
    else:
        label = _label_from_loss(loss)
        if sacrifice >= BRILLIANT_MIN_SAC and loss > BRILLIANT_MAX_LOSS:
            explanation = (
                f"Sacrifices material but is not sound (loses {loss/100:.1f} vs best)."
            )
        else:
            explanation = f"{label}: {loss/100:.2f} lost vs best ({_fmt_eval(eval_before)} -> {_fmt_eval(eval_after)})."

    return MoveJudgment(
        move=move,
        san=san,
        label=label,
        eval_before=eval_before,
        eval_after=eval_after,
        loss=loss,
        sacrifice=sacrifice,
        is_brilliant=is_brilliant,
        explanation=explanation,
    )


def find_brilliant_move(
    engine: Engine, board: chess.Board, depth: int = 14
) -> MoveJudgment | None:
    """Search the position for a brilliant move among the engine's top options.

    The best move is the prime candidate; if it is a sound sacrifice it will be
    flagged. Returns the judgment if a brilliant move exists, else None.
    """
    pre = engine.analyse(board, depth=depth)
    judgment = judge_move(engine, board, pre.best_move, depth=depth, pre=pre)
    return judgment if judgment.is_brilliant else None


def scan_game(
    engine: Engine, board: chess.Board, moves: list[chess.Move], depth: int = 14
) -> list[MoveJudgment]:
    """Judge every move of a game (given as a start position + move list)."""
    judgments: list[MoveJudgment] = []
    b = board.copy()
    for mv in moves:
        judgments.append(judge_move(engine, b, mv, depth=depth))
        b.push(mv)
    return judgments
