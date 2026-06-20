"""Persistent learning file ("experience book").

After a game review, the positions where mistakes were made are analysed at
higher depth and the correct move is stored here, keyed by position. When the
engine later reaches a learned position it recalls the deeper result instead of
re-deriving it with a shallow live search -- so it genuinely improves over time
on the exact positions you got wrong.

This is the same idea as a classical engine "learning file": we don't retrain
weights, we accumulate verified knowledge keyed by Zobrist/EPD position.
"""

from __future__ import annotations

import json
import os
import threading
import time

import chess


class LearningStore:
    def __init__(self, path: str):
        self.path = path
        self._lock = threading.Lock()
        self.data: dict[str, dict] = {}
        self._load()

    def _load(self) -> None:
        if os.path.exists(self.path):
            try:
                with open(self.path, encoding="utf-8") as fh:
                    self.data = json.load(fh)
            except (json.JSONDecodeError, OSError):
                self.data = {}

    def save(self) -> None:
        with self._lock:
            os.makedirs(os.path.dirname(self.path), exist_ok=True)
            tmp = self.path + ".tmp"
            with open(tmp, "w", encoding="utf-8") as fh:
                json.dump(self.data, fh)
            os.replace(tmp, self.path)

    @staticmethod
    def key(board: chess.Board) -> str:
        # EPD = placement + side + castling + ep (no move counters) so the same
        # position reached by any move order / transposition matches.
        return board.epd()

    def get(self, board: chess.Board) -> dict | None:
        return self.data.get(self.key(board))

    def put(self, board: chess.Board, entry: dict) -> bool:
        """Store an entry, keeping whichever analysis is deeper. Returns True if
        this position is newly learned (not just an update to an existing one)."""
        with self._lock:
            k = self.key(board)
            old = self.data.get(k)
            if old is not None and old.get("depth", 0) >= entry.get("depth", 0):
                return False
            entry["ts"] = time.time()
            self.data[k] = entry
            return old is None

    def __len__(self) -> int:
        return len(self.data)
