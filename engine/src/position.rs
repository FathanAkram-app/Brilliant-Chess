//! Board representation: bitboards + mailbox, FEN parsing, make/unmake move,
//! and attack queries used for legality and search.

use crate::tables::*;
use crate::types::*;
use crate::zobrist::{piece_key, zobrist};

// Castling-right bit flags.
pub const CR_WK: u8 = 1;
pub const CR_WQ: u8 = 2;
pub const CR_BK: u8 = 4;
pub const CR_BQ: u8 = 8;

/// State needed to undo a move.
#[derive(Clone, Copy)]
pub struct Undo {
    pub captured: Option<PieceType>,
    pub castling: u8,
    pub ep: Option<Square>,
    pub halfmove: u16,
    pub hash: u64,
}

#[derive(Clone)]
pub struct Position {
    /// Piece bitboards indexed [color][piece_type].
    pub bb: [[Bitboard; 6]; 2],
    /// Per-color occupancy.
    pub occ: [Bitboard; 2],
    /// All occupied squares.
    pub all: Bitboard,
    /// Quick square -> (color, piece) lookup.
    pub mailbox: [Option<(Color, PieceType)>; 64],
    pub side: Color,
    pub castling: u8,
    pub ep: Option<Square>,
    pub halfmove: u16,
    pub fullmove: u16,
    /// Incrementally-maintained Zobrist hash of the position.
    pub hash: u64,
}

impl Position {
    pub fn empty() -> Position {
        Position {
            bb: [[0; 6]; 2],
            occ: [0; 2],
            all: 0,
            mailbox: [None; 64],
            side: Color::White,
            castling: 0,
            ep: None,
            halfmove: 0,
            fullmove: 1,
            hash: 0,
        }
    }

    pub fn startpos() -> Position {
        Position::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap()
    }

    #[inline]
    fn put(&mut self, c: Color, pt: PieceType, sq: Square) {
        let b = 1u64 << sq;
        self.bb[c.index()][pt.index()] |= b;
        self.occ[c.index()] |= b;
        self.all |= b;
        self.mailbox[sq as usize] = Some((c, pt));
        self.hash ^= piece_key(c, pt, sq);
    }

    #[inline]
    fn remove(&mut self, c: Color, pt: PieceType, sq: Square) {
        let b = !(1u64 << sq);
        self.bb[c.index()][pt.index()] &= b;
        self.occ[c.index()] &= b;
        self.all &= b;
        self.mailbox[sq as usize] = None;
        self.hash ^= piece_key(c, pt, sq);
    }

    #[inline]
    fn move_piece(&mut self, c: Color, pt: PieceType, from: Square, to: Square) {
        self.remove(c, pt, from);
        self.put(c, pt, to);
    }

    #[inline]
    pub fn king_sq(&self, c: Color) -> Square {
        self.bb[c.index()][PieceType::King.index()].trailing_zeros() as Square
    }

    /// Is `sq` attacked by any piece of color `by`?
    pub fn is_attacked(&self, sq: Square, by: Color) -> bool {
        let b = by.index();
        // Pawns: squares from which a `by` pawn attacks sq == pawn_attacks of opp color from sq.
        if pawn_attacks(by.opp(), sq) & self.bb[b][PieceType::Pawn.index()] != 0 {
            return true;
        }
        if knight_attacks(sq) & self.bb[b][PieceType::Knight.index()] != 0 {
            return true;
        }
        if king_attacks(sq) & self.bb[b][PieceType::King.index()] != 0 {
            return true;
        }
        let bishops_queens =
            self.bb[b][PieceType::Bishop.index()] | self.bb[b][PieceType::Queen.index()];
        if bishop_attacks(sq, self.all) & bishops_queens != 0 {
            return true;
        }
        let rooks_queens =
            self.bb[b][PieceType::Rook.index()] | self.bb[b][PieceType::Queen.index()];
        if rook_attacks(sq, self.all) & rooks_queens != 0 {
            return true;
        }
        false
    }

    #[inline]
    pub fn in_check(&self, c: Color) -> bool {
        self.is_attacked(self.king_sq(c), c.opp())
    }

    /// Recompute the Zobrist hash from scratch (used to validate incremental updates).
    pub fn compute_hash(&self) -> u64 {
        let z = zobrist();
        let mut h = 0u64;
        for sq in 0..64u8 {
            if let Some((c, pt)) = self.mailbox[sq as usize] {
                h ^= piece_key(c, pt, sq);
            }
        }
        h ^= z.castling[self.castling as usize];
        if let Some(ep) = self.ep {
            h ^= z.ep_file[file_of(ep) as usize];
        }
        if self.side == Color::Black {
            h ^= z.side;
        }
        h
    }

    // ---- FEN ----
    pub fn from_fen(fen: &str) -> Result<Position, String> {
        let mut pos = Position::empty();
        let parts: Vec<&str> = fen.split_whitespace().collect();
        if parts.len() < 4 {
            return Err(format!("FEN needs at least 4 fields, got {}", parts.len()));
        }

        // 1. Piece placement (rank 8 first).
        let mut rank: i32 = 7;
        let mut file: i32 = 0;
        for ch in parts[0].chars() {
            match ch {
                '/' => {
                    rank -= 1;
                    file = 0;
                }
                '1'..='9' => file += ch.to_digit(10).unwrap() as i32,
                _ => {
                    let color = if ch.is_uppercase() {
                        Color::White
                    } else {
                        Color::Black
                    };
                    let pt = match ch.to_ascii_lowercase() {
                        'p' => PieceType::Pawn,
                        'n' => PieceType::Knight,
                        'b' => PieceType::Bishop,
                        'r' => PieceType::Rook,
                        'q' => PieceType::Queen,
                        'k' => PieceType::King,
                        _ => return Err(format!("bad piece char '{}'", ch)),
                    };
                    if !on_board_i(file, rank) {
                        return Err("FEN piece placement out of bounds".into());
                    }
                    pos.put(color, pt, make_square(file as u8, rank as u8));
                    file += 1;
                }
            }
        }

        // 2. Side to move.
        pos.side = match parts[1] {
            "w" => Color::White,
            "b" => Color::Black,
            _ => return Err("bad side to move".into()),
        };

        // 3. Castling rights.
        pos.castling = 0;
        if parts[2] != "-" {
            for ch in parts[2].chars() {
                match ch {
                    'K' => pos.castling |= CR_WK,
                    'Q' => pos.castling |= CR_WQ,
                    'k' => pos.castling |= CR_BK,
                    'q' => pos.castling |= CR_BQ,
                    _ => return Err(format!("bad castling char '{}'", ch)),
                }
            }
        }

        // 4. En passant target square.
        pos.ep = if parts[3] == "-" {
            None
        } else {
            string_to_square(parts[3])
        };

        // 5/6. Halfmove clock and fullmove number (optional).
        pos.halfmove = parts.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
        pos.fullmove = parts.get(5).and_then(|s| s.parse().ok()).unwrap_or(1);

        // Finalize hash: piece keys were added by put(); add state keys now.
        let z = zobrist();
        pos.hash ^= z.castling[pos.castling as usize];
        if let Some(ep) = pos.ep {
            pos.hash ^= z.ep_file[file_of(ep) as usize];
        }
        if pos.side == Color::Black {
            pos.hash ^= z.side;
        }

        Ok(pos)
    }

    pub fn to_fen(&self) -> String {
        let mut s = String::new();
        for rank in (0..8).rev() {
            let mut empty = 0;
            for file in 0..8 {
                let sq = make_square(file, rank);
                if let Some((c, pt)) = self.mailbox[sq as usize] {
                    if empty > 0 {
                        s.push_str(&empty.to_string());
                        empty = 0;
                    }
                    let ch = pt.to_char();
                    s.push(if c == Color::White {
                        ch
                    } else {
                        ch.to_ascii_lowercase()
                    });
                } else {
                    empty += 1;
                }
            }
            if empty > 0 {
                s.push_str(&empty.to_string());
            }
            if rank > 0 {
                s.push('/');
            }
        }
        s.push(' ');
        s.push(if self.side == Color::White { 'w' } else { 'b' });
        s.push(' ');
        if self.castling == 0 {
            s.push('-');
        } else {
            if self.castling & CR_WK != 0 {
                s.push('K');
            }
            if self.castling & CR_WQ != 0 {
                s.push('Q');
            }
            if self.castling & CR_BK != 0 {
                s.push('k');
            }
            if self.castling & CR_BQ != 0 {
                s.push('q');
            }
        }
        s.push(' ');
        match self.ep {
            Some(sq) => s.push_str(&square_to_string(sq)),
            None => s.push('-'),
        }
        s.push_str(&format!(" {} {}", self.halfmove, self.fullmove));
        s
    }

    // ---- make / unmake ----
    pub fn make_move(&mut self, mv: Move) -> Undo {
        let us = self.side;
        let them = us.opp();
        let from = mv.from();
        let to = mv.to();
        let flag = mv.flag();
        let (_, moving_pt) = self.mailbox[from as usize].expect("no piece on from square");

        let undo = Undo {
            captured: None,
            castling: self.castling,
            ep: self.ep,
            halfmove: self.halfmove,
            hash: self.hash,
        };
        let mut captured: Option<PieceType> = None;
        let z = zobrist();

        // Reset ep target; set later if this is a double push.
        let prev_ep = self.ep;
        if let Some(old_ep) = prev_ep {
            self.hash ^= z.ep_file[file_of(old_ep) as usize];
        }
        self.ep = None;

        self.halfmove += 1;
        if moving_pt == PieceType::Pawn {
            self.halfmove = 0;
        }

        // Handle captures (including en passant).
        if mv.is_ep() {
            // Captured pawn sits behind the destination square.
            let cap_sq = if us == Color::White { to - 8 } else { to + 8 };
            self.remove(them, PieceType::Pawn, cap_sq);
            captured = Some(PieceType::Pawn);
            self.halfmove = 0;
            let _ = prev_ep;
        } else if mv.is_capture() {
            let (_, cpt) = self.mailbox[to as usize].expect("capture flag but empty target");
            self.remove(them, cpt, to);
            captured = Some(cpt);
            self.halfmove = 0;
        }

        // Move the piece (handle promotion by swapping piece type at destination).
        if let Some(promo) = mv.promo_piece() {
            self.remove(us, PieceType::Pawn, from);
            self.put(us, promo, to);
        } else {
            self.move_piece(us, moving_pt, from, to);
        }

        // Castling: move the rook too.
        if flag == FLAG_KING_CASTLE {
            let (rf, rt) = if us == Color::White { (7, 5) } else { (63, 61) };
            self.move_piece(us, PieceType::Rook, rf, rt);
        } else if flag == FLAG_QUEEN_CASTLE {
            let (rf, rt) = if us == Color::White { (0, 3) } else { (56, 59) };
            self.move_piece(us, PieceType::Rook, rf, rt);
        }

        // Set en passant target on a double pawn push.
        if flag == FLAG_DOUBLE_PAWN {
            let ep_sq = if us == Color::White { to - 8 } else { to + 8 };
            self.ep = Some(ep_sq);
            self.hash ^= z.ep_file[file_of(ep_sq) as usize];
        }

        // Update castling rights based on from/to squares touched.
        let old_castling = self.castling;
        self.castling &= castling_mask(from);
        self.castling &= castling_mask(to);
        if old_castling != self.castling {
            self.hash ^= z.castling[old_castling as usize] ^ z.castling[self.castling as usize];
        }

        if us == Color::Black {
            self.fullmove += 1;
        }
        self.side = them;
        self.hash ^= z.side;

        let mut u = undo;
        u.captured = captured;
        u
    }

    pub fn unmake_move(&mut self, mv: Move, undo: Undo) {
        let them = self.side; // side that didn't move
        let us = them.opp(); // side that moved
        let from = mv.from();
        let to = mv.to();
        let flag = mv.flag();

        self.side = us;
        self.castling = undo.castling;
        self.ep = undo.ep;
        self.halfmove = undo.halfmove;
        if us == Color::Black {
            self.fullmove -= 1;
        }

        // Undo the piece move / promotion.
        if let Some(promo) = mv.promo_piece() {
            self.remove(us, promo, to);
            self.put(us, PieceType::Pawn, from);
        } else {
            let (_, pt) = self.mailbox[to as usize].expect("no piece on to during unmake");
            self.move_piece(us, pt, to, from);
        }

        // Undo castling rook move.
        if flag == FLAG_KING_CASTLE {
            let (rf, rt) = if us == Color::White { (7, 5) } else { (63, 61) };
            self.move_piece(us, PieceType::Rook, rt, rf);
        } else if flag == FLAG_QUEEN_CASTLE {
            let (rf, rt) = if us == Color::White { (0, 3) } else { (56, 59) };
            self.move_piece(us, PieceType::Rook, rt, rf);
        }

        // Restore captured piece.
        if mv.is_ep() {
            let cap_sq = if us == Color::White { to - 8 } else { to + 8 };
            self.put(them, PieceType::Pawn, cap_sq);
        } else if let Some(cpt) = undo.captured {
            self.put(them, cpt, to);
        }

        // The put/remove calls above scrambled the hash; restore it wholesale.
        self.hash = undo.hash;
    }
}

#[inline]
fn on_board_i(file: i32, rank: i32) -> bool {
    (0..8).contains(&file) && (0..8).contains(&rank)
}

/// Mask of castling rights to KEEP when the given square is moved from or to.
fn castling_mask(sq: Square) -> u8 {
    match sq {
        0 => !CR_WQ,  // a1 rook
        4 => !(CR_WK | CR_WQ), // e1 king
        7 => !CR_WK,  // h1 rook
        56 => !CR_BQ, // a8 rook
        60 => !(CR_BK | CR_BQ), // e8 king
        63 => !CR_BK, // h8 rook
        _ => 0xff,
    }
}
