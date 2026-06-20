//! Precomputed attack tables for leapers (knight, king, pawn) and on-the-fly
//! ray-based attacks for sliding pieces (bishop, rook, queen).

use crate::types::*;
use std::sync::OnceLock;

struct Tables {
    knight: [Bitboard; 64],
    king: [Bitboard; 64],
    pawn: [[Bitboard; 64]; 2], // [color][square]
}

static TABLES: OnceLock<Tables> = OnceLock::new();

fn tables() -> &'static Tables {
    TABLES.get_or_init(build_tables)
}

#[inline]
fn bit(sq: i32) -> Bitboard {
    1u64 << sq
}

/// True if (file,rank) is on the board.
#[inline]
fn on_board(file: i32, rank: i32) -> bool {
    (0..8).contains(&file) && (0..8).contains(&rank)
}

fn build_tables() -> Tables {
    let mut knight = [0u64; 64];
    let mut king = [0u64; 64];
    let mut pawn = [[0u64; 64]; 2];

    let knight_deltas: [(i32, i32); 8] = [
        (1, 2),
        (2, 1),
        (2, -1),
        (1, -2),
        (-1, -2),
        (-2, -1),
        (-2, 1),
        (-1, 2),
    ];
    let king_deltas: [(i32, i32); 8] = [
        (1, 0),
        (1, 1),
        (0, 1),
        (-1, 1),
        (-1, 0),
        (-1, -1),
        (0, -1),
        (1, -1),
    ];

    for sq in 0..64i32 {
        let f = file_of(sq as u8) as i32;
        let r = rank_of(sq as u8) as i32;

        for (df, dr) in knight_deltas {
            if on_board(f + df, r + dr) {
                knight[sq as usize] |= bit((r + dr) * 8 + (f + df));
            }
        }
        for (df, dr) in king_deltas {
            if on_board(f + df, r + dr) {
                king[sq as usize] |= bit((r + dr) * 8 + (f + df));
            }
        }
        // White pawn attacks: up-left and up-right.
        for df in [-1, 1] {
            if on_board(f + df, r + 1) {
                pawn[Color::White.index()][sq as usize] |= bit((r + 1) * 8 + (f + df));
            }
            if on_board(f + df, r - 1) {
                pawn[Color::Black.index()][sq as usize] |= bit((r - 1) * 8 + (f + df));
            }
        }
    }

    Tables { knight, king, pawn }
}

#[inline]
pub fn knight_attacks(sq: Square) -> Bitboard {
    tables().knight[sq as usize]
}
#[inline]
pub fn king_attacks(sq: Square) -> Bitboard {
    tables().king[sq as usize]
}
#[inline]
pub fn pawn_attacks(color: Color, sq: Square) -> Bitboard {
    tables().pawn[color.index()][sq as usize]
}

const BISHOP_DIRS: [(i32, i32); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];
const ROOK_DIRS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

#[inline]
fn ray_attacks(sq: Square, occ: Bitboard, dirs: &[(i32, i32); 4]) -> Bitboard {
    let mut attacks = 0u64;
    let f0 = file_of(sq) as i32;
    let r0 = rank_of(sq) as i32;
    for &(df, dr) in dirs {
        let mut f = f0 + df;
        let mut r = r0 + dr;
        while on_board(f, r) {
            let s = (r * 8 + f) as u32;
            attacks |= 1u64 << s;
            if occ & (1u64 << s) != 0 {
                break; // blocked (this square itself is included)
            }
            f += df;
            r += dr;
        }
    }
    attacks
}

#[inline]
pub fn bishop_attacks(sq: Square, occ: Bitboard) -> Bitboard {
    ray_attacks(sq, occ, &BISHOP_DIRS)
}
#[inline]
pub fn rook_attacks(sq: Square, occ: Bitboard) -> Bitboard {
    ray_attacks(sq, occ, &ROOK_DIRS)
}
#[inline]
pub fn queen_attacks(sq: Square, occ: Bitboard) -> Bitboard {
    ray_attacks(sq, occ, &BISHOP_DIRS) | ray_attacks(sq, occ, &ROOK_DIRS)
}
