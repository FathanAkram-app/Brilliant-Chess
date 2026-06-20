//! Pseudo-legal move generation followed by a legality filter
//! (make the move, reject if it leaves the mover's king in check).

use crate::position::*;
use crate::tables::*;
use crate::types::*;

pub type MoveList = Vec<Move>;

/// Iterate set bits of a bitboard, yielding square indices.
#[inline]
fn pop_lsb(bb: &mut Bitboard) -> Square {
    let sq = bb.trailing_zeros() as Square;
    *bb &= *bb - 1;
    sq
}

/// Generate all fully legal moves for the side to move.
pub fn generate_legal(pos: &mut Position) -> MoveList {
    let mut pseudo = MoveList::with_capacity(48);
    generate_pseudo(pos, &mut pseudo);

    let us = pos.side;
    let mut legal = MoveList::with_capacity(pseudo.len());
    for mv in pseudo {
        let undo = pos.make_move(mv);
        // After make_move, side flipped; check the mover's king.
        if !pos.is_attacked(pos.king_sq(us), us.opp()) {
            legal.push(mv);
        }
        pos.unmake_move(mv, undo);
    }
    legal
}

fn generate_pseudo(pos: &Position, list: &mut MoveList) {
    let us = pos.side;
    let them = us.opp();
    let own = pos.occ[us.index()];
    let enemy = pos.occ[them.index()];
    let all = pos.all;

    gen_pawns(pos, us, them, enemy, all, list);
    gen_knights(pos, us, own, enemy, list);
    gen_bishops_rooks_queens(pos, us, own, enemy, all, list);
    gen_king(pos, us, own, enemy, list);
    gen_castling(pos, us, them, all, list);
}

fn gen_pawns(
    pos: &Position,
    us: Color,
    them: Color,
    enemy: Bitboard,
    all: Bitboard,
    list: &mut MoveList,
) {
    let pawns = pos.bb[us.index()][PieceType::Pawn.index()];
    let mut bb = pawns;
    while bb != 0 {
        let from = pop_lsb(&mut bb);
        let r = rank_of(from);
        let forward: i32 = if us == Color::White { 8 } else { -8 };
        let one = (from as i32 + forward) as i32;
        let promo_rank = if us == Color::White { 6 } else { 1 }; // rank from which a push promotes
        let start_rank = if us == Color::White { 1 } else { 6 };

        // Single push.
        if (0..64).contains(&one) && (all & (1u64 << one)) == 0 {
            let to = one as Square;
            if r == promo_rank {
                push_promotions(from, to, false, list);
            } else {
                list.push(Move::new(from, to, FLAG_QUIET));
                // Double push.
                if r == start_rank {
                    let two = (one + forward) as i32;
                    if (all & (1u64 << two)) == 0 {
                        list.push(Move::new(from, two as Square, FLAG_DOUBLE_PAWN));
                    }
                }
            }
        }

        // Captures (including promotion captures and en passant).
        let mut atk = pawn_attacks(us, from) & enemy;
        while atk != 0 {
            let to = pop_lsb(&mut atk);
            if r == promo_rank {
                push_promotions(from, to, true, list);
            } else {
                list.push(Move::new(from, to, FLAG_CAPTURE));
            }
        }
        if let Some(ep) = pos.ep {
            if pawn_attacks(us, from) & (1u64 << ep) != 0 {
                list.push(Move::new(from, ep, FLAG_EP_CAPTURE));
            }
        }
    }
    let _ = them;
}

#[inline]
fn push_promotions(from: Square, to: Square, capture: bool, list: &mut MoveList) {
    if capture {
        list.push(Move::new(from, to, FLAG_PROMO_Q_CAP));
        list.push(Move::new(from, to, FLAG_PROMO_R_CAP));
        list.push(Move::new(from, to, FLAG_PROMO_B_CAP));
        list.push(Move::new(from, to, FLAG_PROMO_N_CAP));
    } else {
        list.push(Move::new(from, to, FLAG_PROMO_Q));
        list.push(Move::new(from, to, FLAG_PROMO_R));
        list.push(Move::new(from, to, FLAG_PROMO_B));
        list.push(Move::new(from, to, FLAG_PROMO_N));
    }
}

fn gen_knights(pos: &Position, us: Color, own: Bitboard, enemy: Bitboard, list: &mut MoveList) {
    let mut bb = pos.bb[us.index()][PieceType::Knight.index()];
    while bb != 0 {
        let from = pop_lsb(&mut bb);
        let mut targets = knight_attacks(from) & !own;
        emit_targets(from, &mut targets, enemy, list);
    }
}

fn gen_bishops_rooks_queens(
    pos: &Position,
    us: Color,
    own: Bitboard,
    enemy: Bitboard,
    all: Bitboard,
    list: &mut MoveList,
) {
    let u = us.index();
    let mut bishops = pos.bb[u][PieceType::Bishop.index()];
    while bishops != 0 {
        let from = pop_lsb(&mut bishops);
        let mut targets = bishop_attacks(from, all) & !own;
        emit_targets(from, &mut targets, enemy, list);
    }
    let mut rooks = pos.bb[u][PieceType::Rook.index()];
    while rooks != 0 {
        let from = pop_lsb(&mut rooks);
        let mut targets = rook_attacks(from, all) & !own;
        emit_targets(from, &mut targets, enemy, list);
    }
    let mut queens = pos.bb[u][PieceType::Queen.index()];
    while queens != 0 {
        let from = pop_lsb(&mut queens);
        let mut targets = queen_attacks(from, all) & !own;
        emit_targets(from, &mut targets, enemy, list);
    }
}

fn gen_king(pos: &Position, us: Color, own: Bitboard, enemy: Bitboard, list: &mut MoveList) {
    let from = pos.king_sq(us);
    let mut targets = king_attacks(from) & !own;
    emit_targets(from, &mut targets, enemy, list);
}

#[inline]
fn emit_targets(from: Square, targets: &mut Bitboard, enemy: Bitboard, list: &mut MoveList) {
    while *targets != 0 {
        let to = pop_lsb(targets);
        let flag = if enemy & (1u64 << to) != 0 {
            FLAG_CAPTURE
        } else {
            FLAG_QUIET
        };
        list.push(Move::new(from, to, flag));
    }
}

fn gen_castling(pos: &Position, us: Color, them: Color, all: Bitboard, list: &mut MoveList) {
    // Cannot castle out of check.
    let king = pos.king_sq(us);
    if pos.is_attacked(king, them) {
        return;
    }
    if us == Color::White {
        // King on e1 (4). Kingside: f1,g1 empty & not attacked, rook h1.
        if pos.castling & CR_WK != 0
            && (all & ((1 << 5) | (1 << 6))) == 0
            && !pos.is_attacked(5, them)
            && !pos.is_attacked(6, them)
        {
            list.push(Move::new(4, 6, FLAG_KING_CASTLE));
        }
        if pos.castling & CR_WQ != 0
            && (all & ((1 << 1) | (1 << 2) | (1 << 3))) == 0
            && !pos.is_attacked(3, them)
            && !pos.is_attacked(2, them)
        {
            list.push(Move::new(4, 2, FLAG_QUEEN_CASTLE));
        }
    } else {
        // King on e8 (60).
        if pos.castling & CR_BK != 0
            && (all & ((1 << 61) | (1 << 62))) == 0
            && !pos.is_attacked(61, them)
            && !pos.is_attacked(62, them)
        {
            list.push(Move::new(60, 62, FLAG_KING_CASTLE));
        }
        if pos.castling & CR_BQ != 0
            && (all & ((1 << 57) | (1 << 58) | (1 << 59))) == 0
            && !pos.is_attacked(59, them)
            && !pos.is_attacked(58, them)
        {
            list.push(Move::new(60, 58, FLAG_QUEEN_CASTLE));
        }
    }
}
