//! One side of a book as a widget shows it: its best levels, each a level
//! of its own or merged into price groups (the old `packMetadataSide`).

use crate::data::ferris::BookLevel;

/// A side of a book.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    Bid,
    Ask,
}

/// One level a widget shows: a price of one side, and what rests at it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Packed {
    /// The price it shows at: its group's when prices are grouped.
    pub price: f64,
    pub amount: f64,
    /// Price times amount, summed over what it merges.
    pub total: f64,
    /// The amounts from its side's best price through it.
    pub cumulative: f64,
}

/// One side of a book as it is shown: its best `depth` levels, each a
/// level of its own, or merged while they share a price group of
/// `grouping` (zero: raw prices). A level with no amount is left out
/// unless `empty` keeps it.
pub(crate) fn pack(
    side_levels: &[BookLevel],
    depth: usize,
    grouping: f64,
    side: Side,
    empty: bool,
) -> Vec<Packed> {
    let finite = |value: f64| if value.is_finite() { value } else { 0.0 };
    let mut packed = Vec::new();
    let mut running = 0.0;
    let mut group: Option<Packed> = None;
    for level in side_levels.iter().take(depth) {
        if !level.price.is_finite() {
            continue;
        }
        let amount = finite(level.amount);
        if !empty && amount <= 0.0 {
            continue;
        }
        if amount >= 0.0 {
            running += amount;
        }
        let cumulative = if level.cumulative.is_finite() {
            level.cumulative.max(0.0)
        } else {
            running
        };
        let price = grouped(level.price, grouping, side);
        if group
            .as_ref()
            .is_some_and(|group| grouping == 0.0 || group.price != price)
        {
            packed.extend(group.take());
        }
        let group = group.get_or_insert(Packed {
            price,
            amount: 0.0,
            total: 0.0,
            cumulative: 0.0,
        });
        group.amount += amount;
        group.total += finite(level.total);
        group.cumulative = cumulative;
    }
    packed.extend(group);
    packed
}

/// `price` in its group of `grouping`: a bid's at or below it, an ask's at
/// or above it, so a group never shows a better price than it holds. A
/// price a rounding error away from a group's edge is on it.
fn grouped(price: f64, grouping: f64, side: Side) -> f64 {
    if grouping == 0.0 {
        return price;
    }
    let quotient = price / grouping;
    let nearest = round(quotient);
    let tolerance = f64::EPSILON * quotient.abs().max(1.0);
    let stable = if (quotient - nearest).abs() <= tolerance {
        nearest
    } else {
        quotient
    };
    let edge = match side {
        Side::Bid => stable.floor(),
        Side::Ask => stable.ceil(),
    } * grouping;
    // Never negative zero.
    if edge == 0.0 { 0.0 } else { edge }
}

/// `value` to the nearest whole number, halves up, as JavaScript's
/// `Math.round` (Rust's rounds halves away from zero).
pub(crate) fn round(value: f64) -> f64 {
    (value + 0.5).floor()
}
