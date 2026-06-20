//! Zobrist hashing keys. Deterministic (fixed seed) so hashes are stable
//! across runs. Piece keys are XORed in/out by Position::put/remove; the
//! castling/ep/side keys are handled in make/unmake.

use crate::types::*;
use std::sync::OnceLock;

pub struct Zobrist {
    pub pieces: [[[u64; 64]; 6]; 2], // [color][piece][square]
    pub castling: [u64; 16],
    pub ep_file: [u64; 8],
    pub side: u64,
}

static ZOBRIST: OnceLock<Zobrist> = OnceLock::new();

/// SplitMix64 PRNG for reproducible key generation.
struct SplitMix64 {
    state: u64,
}
impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
}

pub fn zobrist() -> &'static Zobrist {
    ZOBRIST.get_or_init(|| {
        let mut rng = SplitMix64 {
            state: 0x1234_5678_9ABC_DEF0,
        };
        let mut pieces = [[[0u64; 64]; 6]; 2];
        for c in 0..2 {
            for p in 0..6 {
                for s in 0..64 {
                    pieces[c][p][s] = rng.next();
                }
            }
        }
        let mut castling = [0u64; 16];
        for c in castling.iter_mut() {
            *c = rng.next();
        }
        let mut ep_file = [0u64; 8];
        for e in ep_file.iter_mut() {
            *e = rng.next();
        }
        let side = rng.next();
        Zobrist {
            pieces,
            castling,
            ep_file,
            side,
        }
    })
}

#[inline]
pub fn piece_key(c: Color, pt: PieceType, sq: Square) -> u64 {
    zobrist().pieces[c.index()][pt.index()][sq as usize]
}
