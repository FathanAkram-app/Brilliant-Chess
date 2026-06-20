"""Driver for the Rust UCI engine, built on python-chess.

Provides a thin, convenient wrapper for analysing positions: best move,
evaluation (centipawns, mate-aware), the principal variation (the engine's
prediction of best play for *both* sides), and a full rollout to game end.
"""

from __future__ import annotations

import os
import platform
from dataclasses import dataclass, field

import chess
import chess.engine

# Centipawn value assigned to a forced mate when flattening PovScore -> int.
MATE_SCORE = 100_000


def default_engine_path() -> str:
    """Locate the compiled Rust engine binary next to this package."""
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    exe = "chess_engine.exe" if platform.system() == "Windows" else "chess_engine"
    path = os.path.join(root, "engine", "target", "release", exe)
    if not os.path.exists(path):
        raise FileNotFoundError(
            f"Engine binary not found at {path}.\n"
            "Build it first:  cargo build --release  (inside the engine/ directory)"
        )
    return path


@dataclass
class Analysis:
    """Result of analysing a single position."""

    best_move: chess.Move
    score_cp: int          # centipawns from the side-to-move's perspective
    mate: int | None       # mate in N (positive = side to move mates), else None
    pv: list[chess.Move]   # predicted best line for both sides
    depth: int


@dataclass
class Engine:
    """Context-managed handle to the UCI engine process."""

    path: str = field(default_factory=default_engine_path)
    hash_mb: int = 128
    verbose: bool = False  # stream the engine's per-depth search output
    _engine: chess.engine.SimpleEngine | None = field(default=None, repr=False)

    def __enter__(self) -> "Engine":
        self._engine = chess.engine.SimpleEngine.popen_uci(self.path)
        try:
            self._engine.configure({"Hash": self.hash_mb})
        except chess.engine.EngineError:
            pass  # option may be unsupported; ignore
        return self

    def __exit__(self, *exc) -> None:
        if self._engine is not None:
            self._engine.quit()
            self._engine = None

    def _limit(self, depth: int | None, movetime: float | None) -> chess.engine.Limit:
        kw: dict = {}
        if depth is not None:
            kw["depth"] = depth
        if movetime is not None:
            kw["time"] = movetime  # whichever is reached first stops the search
        if not kw:
            kw["depth"] = 12
        return chess.engine.Limit(**kw)

    def analyse(
        self,
        board: chess.Board,
        depth: int | None = 12,
        movetime: float | None = None,
        verbose: bool | None = None,
        label: str | None = None,
        root_moves: list[chess.Move] | None = None,
    ) -> Analysis:
        """Analyse a position and return best move, score, and predicted line.

        When verbose (or self.verbose), stream each search depth's result live
        so you can watch the engine think. `root_moves` restricts the search to
        a subset of moves (used to find the second-best move).
        """
        assert self._engine is not None, "use Engine inside a `with` block"
        verbose = self.verbose if verbose is None else verbose

        if not verbose:
            info = self._engine.analyse(
                board, self._limit(depth, movetime), root_moves=root_moves
            )
            return self._to_analysis(board, info, depth)

        # Streaming mode: iterate info lines as the engine produces them.
        if label:
            print(label, flush=True)
        last: dict = {}
        with self._engine.analysis(board, self._limit(depth, movetime)) as analysis:
            for info in analysis:
                if "pv" in info and "score" in info and "depth" in info:
                    last = info
                    self._print_search_info(board, info)
        if not last:
            last = analysis.info
        return self._to_analysis(board, last, depth)

    def _to_analysis(self, board: chess.Board, info: dict, depth: int | None) -> Analysis:
        pov = info["score"].pov(board.turn)
        pv = info.get("pv", []) or []
        best = pv[0] if pv else next(iter(board.legal_moves))
        return Analysis(
            best_move=best,
            score_cp=pov.score(mate_score=MATE_SCORE),
            mate=pov.mate(),
            pv=list(pv),
            depth=info.get("depth", depth or 0),
        )

    @staticmethod
    def _print_search_info(board: chess.Board, info: dict) -> None:
        """Render one UCI info line in human-readable form (SAN, score, stats)."""
        pov = info["score"].pov(board.turn)
        mate = pov.mate()
        score = f"#{mate}" if mate is not None else f"{pov.score()/100:+.2f}"
        # Render the predicted line in SAN.
        san, b = [], board.copy()
        for mv in info.get("pv", [])[:12]:
            try:
                san.append(b.san(mv))
                b.push(mv)
            except (ValueError, AssertionError):
                break
        nodes = info.get("nodes", 0)
        nps = info.get("nps", 0)
        print(
            f"  depth {info['depth']:>2}  {score:>7}  "
            f"nodes {nodes:>9}  nps {nps:>8}  pv: {' '.join(san)}",
            flush=True,
        )

    def evaluate(
        self, board: chess.Board, depth: int | None = 12, movetime: float | None = None
    ) -> int:
        """Centipawn score from the side-to-move's perspective (mate-aware)."""
        if board.is_game_over():
            if board.is_checkmate():
                return -MATE_SCORE  # side to move is mated
            return 0  # stalemate / draw
        # Internal helper: never stream (would double the verbose output).
        return self.analyse(board, depth, movetime, verbose=False).score_cp

    def predict_line(
        self,
        board: chess.Board,
        depth: int | None = 16,
        movetime: float | None = None,
    ) -> list[chess.Move]:
        """The principal variation: predicted best moves for both players."""
        return self.analyse(board, depth, movetime).pv

    def rollout_to_end(
        self,
        board: chess.Board,
        depth: int = 12,
        max_plies: int = 200,
    ) -> list[chess.Move]:
        """Play out the whole game by repeatedly choosing the engine's best move.

        This literally 'predicts the game to the end': it keeps moving until
        checkmate, stalemate, or another draw condition is reached.
        """
        b = board.copy()
        moves: list[chess.Move] = []
        for _ in range(max_plies):
            if b.is_game_over():
                break
            mv = self.analyse(b, depth=depth).best_move
            moves.append(mv)
            b.push(mv)
        return moves
