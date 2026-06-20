//! Core type definitions: colors, piece types, squares, and the compact Move encoding.

pub type Bitboard = u64;
/// Square index 0..=63, a1 = 0, b1 = 1, ... h8 = 63.
pub type Square = u8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Color {
    White = 0,
    Black = 1,
}

impl Color {
    #[inline]
    pub fn index(self) -> usize {
        self as usize
    }
    #[inline]
    pub fn opp(self) -> Color {
        match self {
            Color::White => Color::Black,
            Color::Black => Color::White,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PieceType {
    Pawn = 0,
    Knight = 1,
    Bishop = 2,
    Rook = 3,
    Queen = 4,
    King = 5,
}

impl PieceType {
    #[inline]
    pub fn index(self) -> usize {
        self as usize
    }
    pub fn from_index(i: usize) -> PieceType {
        match i {
            0 => PieceType::Pawn,
            1 => PieceType::Knight,
            2 => PieceType::Bishop,
            3 => PieceType::Rook,
            4 => PieceType::Queen,
            5 => PieceType::King,
            _ => unreachable!(),
        }
    }
    /// The character used in FEN / SAN for this piece type (uppercase).
    pub fn to_char(self) -> char {
        match self {
            PieceType::Pawn => 'P',
            PieceType::Knight => 'N',
            PieceType::Bishop => 'B',
            PieceType::Rook => 'R',
            PieceType::Queen => 'Q',
            PieceType::King => 'K',
        }
    }
}

pub const ALL_PIECE_TYPES: [PieceType; 6] = [
    PieceType::Pawn,
    PieceType::Knight,
    PieceType::Bishop,
    PieceType::Rook,
    PieceType::Queen,
    PieceType::King,
];

// ---- Square helpers ----
#[inline]
pub fn file_of(sq: Square) -> u8 {
    sq & 7
}
#[inline]
pub fn rank_of(sq: Square) -> u8 {
    sq >> 3
}
#[inline]
pub fn make_square(file: u8, rank: u8) -> Square {
    (rank << 3) | file
}

/// Convert a square index to algebraic notation, e.g. 0 -> "a1".
pub fn square_to_string(sq: Square) -> String {
    let f = (b'a' + file_of(sq)) as char;
    let r = (b'1' + rank_of(sq)) as char;
    format!("{}{}", f, r)
}

/// Parse algebraic notation like "e4" into a square index.
pub fn string_to_square(s: &str) -> Option<Square> {
    let b = s.as_bytes();
    if b.len() != 2 {
        return None;
    }
    let file = b[0].wrapping_sub(b'a');
    let rank = b[1].wrapping_sub(b'1');
    if file > 7 || rank > 7 {
        return None;
    }
    Some(make_square(file, rank))
}

// ---- Move encoding (16 bits): from(6) | to(6) | flag(4) ----
// Flags follow the common Chess Programming Wiki scheme.
pub const FLAG_QUIET: u16 = 0;
pub const FLAG_DOUBLE_PAWN: u16 = 1;
pub const FLAG_KING_CASTLE: u16 = 2;
pub const FLAG_QUEEN_CASTLE: u16 = 3;
pub const FLAG_CAPTURE: u16 = 4;
pub const FLAG_EP_CAPTURE: u16 = 5;
pub const FLAG_PROMO_N: u16 = 8;
pub const FLAG_PROMO_B: u16 = 9;
pub const FLAG_PROMO_R: u16 = 10;
pub const FLAG_PROMO_Q: u16 = 11;
pub const FLAG_PROMO_N_CAP: u16 = 12;
pub const FLAG_PROMO_B_CAP: u16 = 13;
pub const FLAG_PROMO_R_CAP: u16 = 14;
pub const FLAG_PROMO_Q_CAP: u16 = 15;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Move(pub u16);

impl Move {
    #[inline]
    pub fn new(from: Square, to: Square, flag: u16) -> Move {
        Move((from as u16) | ((to as u16) << 6) | (flag << 12))
    }
    #[inline]
    pub fn from(self) -> Square {
        (self.0 & 0x3f) as Square
    }
    #[inline]
    pub fn to(self) -> Square {
        ((self.0 >> 6) & 0x3f) as Square
    }
    #[inline]
    pub fn flag(self) -> u16 {
        self.0 >> 12
    }
    #[inline]
    pub fn is_capture(self) -> bool {
        // flags with bit 2 (value 4) set: 4,5,6,7,12,13,14,15
        (self.flag() & 4) != 0
    }
    #[inline]
    pub fn is_promotion(self) -> bool {
        self.flag() >= FLAG_PROMO_N
    }
    #[inline]
    pub fn is_ep(self) -> bool {
        self.flag() == FLAG_EP_CAPTURE
    }
    #[inline]
    pub fn is_castle(self) -> bool {
        matches!(self.flag(), FLAG_KING_CASTLE | FLAG_QUEEN_CASTLE)
    }
    /// Promotion target piece type, if this is a promotion.
    pub fn promo_piece(self) -> Option<PieceType> {
        match self.flag() {
            FLAG_PROMO_N | FLAG_PROMO_N_CAP => Some(PieceType::Knight),
            FLAG_PROMO_B | FLAG_PROMO_B_CAP => Some(PieceType::Bishop),
            FLAG_PROMO_R | FLAG_PROMO_R_CAP => Some(PieceType::Rook),
            FLAG_PROMO_Q | FLAG_PROMO_Q_CAP => Some(PieceType::Queen),
            _ => None,
        }
    }
    pub const NONE: Move = Move(0);
    #[inline]
    pub fn is_none(self) -> bool {
        self.0 == 0
    }

    /// Long algebraic / UCI notation, e.g. "e2e4", "e7e8q".
    pub fn to_uci(self) -> String {
        let mut s = String::new();
        s.push_str(&square_to_string(self.from()));
        s.push_str(&square_to_string(self.to()));
        if let Some(pt) = self.promo_piece() {
            s.push(pt.to_char().to_ascii_lowercase());
        }
        s
    }
}

impl std::fmt::Debug for Move {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_uci())
    }
}
