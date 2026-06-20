"""Static Exchange Evaluation (SEE): how much material a side wins by
initiating captures on a given square, assuming both sides play the
least-valuable-attacker capture sequence.

Used by the brilliancy detector to decide whether a move *sacrifices*
material (the opponent can win material by capturing) despite the engine
still judging the position favourable.
"""

from __future__ import annotations

import chess

PIECE_VALUE = {
    chess.PAWN: 100,
    chess.KNIGHT: 320,
    chess.BISHOP: 330,
    chess.ROOK: 500,
    chess.QUEEN: 900,
    chess.KING: 10_000,
}


def _value(piece: chess.Piece | None) -> int:
    return PIECE_VALUE[piece.piece_type] if piece else 0


def see_on_square(board: chess.Board, square: int, side: chess.Color) -> int:
    """Material `side` gains by capturing on `square` (and the recapture chain).

    Non-negative: a side never captures if doing so loses material. Ignores
    pins/legality (standard for SEE) and treats it as a pseudo-legal swap-off.
    """
    target = board.piece_at(square)
    if target is None:
        return 0  # nothing to capture (en passant not modelled here)

    attackers = board.attackers(side, square)
    if not attackers:
        return 0

    # Least valuable attacker initiates the capture.
    from_sq = min(attackers, key=lambda s: _value(board.piece_at(s)))
    captured_value = _value(target)

    board_copy = board.copy(stack=False)
    promo = None
    moving = board_copy.piece_at(from_sq)
    if moving and moving.piece_type == chess.PAWN and chess.square_rank(square) in (0, 7):
        promo = chess.QUEEN
    board_copy.remove_piece_at(from_sq)
    board_copy.set_piece_at(square, chess.Piece(promo or moving.piece_type, side))

    # Recursively, the opponent may recapture on the same square.
    gain = captured_value - see_on_square(board_copy, square, not side)
    return max(0, gain)


def best_opponent_capture(board: chess.Board) -> int:
    """Max material the side to move can win via an immediate capture sequence.

    Scans every enemy piece's square; returns the best SEE the side to move
    can achieve. Used right after a candidate sacrifice to measure how much
    material the defender can grab back.
    """
    side = board.turn
    best = 0
    for square in chess.scan_forward(board.occupied_co[not side]):
        if board.attackers(side, square):
            best = max(best, see_on_square(board, square, side))
    return best
