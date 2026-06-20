"""Opening book lookup via a Polyglot (.bin) book.

In the opening, known theory is looked up instantly instead of being searched
by the engine. A Polyglot book is indexed by Zobrist hash, so a probe is
effectively free and needs no network connection.
"""

from __future__ import annotations

import os

import chess
import chess.polyglot


class OpeningBook:
    def __init__(self, path: str | None):
        self.path = path if (path and os.path.exists(path)) else None

    def available(self) -> bool:
        return self.path is not None

    def probe(self, board: chess.Board, max_moves: int = 6) -> dict | None:
        """Return the best book move plus popular alternatives, or None.

        Result: {"best": chess.Move, "moves": [{san, uci, weight, pct}, ...]}
        where pct is the move's share of total weight (rough popularity).
        """
        if not self.path:
            return None
        try:
            with chess.polyglot.open_reader(self.path) as reader:
                entries = list(reader.find_all(board))
        except (IndexError, OSError, ValueError):
            return None
        if not entries:
            return None

        entries.sort(key=lambda e: -e.weight)
        total = sum(e.weight for e in entries) or 1
        moves = [
            {
                "san": board.san(e.move),
                "uci": e.move.uci(),
                "weight": e.weight,
                "pct": round(100 * e.weight / total, 1),
            }
            for e in entries[:max_moves]
        ]
        return {"best": entries[0].move, "moves": moves}
