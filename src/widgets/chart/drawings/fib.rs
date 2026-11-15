//! The Fibonacci retracement: the levels a move from its first point to
//! its second may retrace to. Each is a line across the move, from one
//! point's time to the other's, named by its ratio and its price: 0 at the
//! second point (nothing retraced), 1 at the first (all of it). A dashed
//! line joins the two points.

use super::shape::{Brush, Shape, Spot, segment_distance};
use crate::format::price_text;

/// The ratios of the move a level is at.
pub(super) const RATIOS: [f64; 7] = [0.0, 0.236, 0.382, 0.5, 0.618, 0.786, 1.0];
/// A level's name is written this far above its line, and in from its
/// left end.
const NAME_ABOVE: f64 = 7.0;
const NAME_IN: f64 = 2.0;

/// One level of a retracement: its ratio, its price, and its line across
/// the plot.
struct Level {
    ratio: f64,
    price: f64,
    left: (f64, f64),
    right: (f64, f64),
}

/// The levels of the move from `from` to `to`, from 0 to 1.
fn levels(from: &Spot, to: &Spot) -> impl Iterator<Item = Level> {
    let (left, right) = (from.x.min(to.x), from.x.max(to.x));
    let (start, end) = (from.anchor.price(), to.anchor.price());
    let (from_y, to_y) = (from.y, to.y);
    RATIOS.into_iter().map(move |ratio| {
        // On whole points, so each line is one sharp row.
        let y = (to_y + (from_y - to_y) * ratio).floor() + 0.5;
        Level {
            ratio,
            price: end + (start - end) * ratio,
            left: (left, y),
            right: (right, y),
        }
    })
}

pub(super) struct Fib;

impl Shape for Fib {
    fn draw(&self, brush: &mut Brush<'_>, spots: &[Spot]) {
        let [from, to] = spots else {
            return;
        };
        brush.dashes(from.place(), to.place(), brush.ink.dim);
        for level in levels(from, to) {
            brush.line(level.left, level.right, brush.ink.line);
            brush.text(
                &format!("{} ({})", level.ratio, price_text(level.price)),
                level.left.0 + NAME_IN,
                level.left.1 - NAME_ABOVE,
                brush.ink.text,
            );
        }
    }

    fn distance(&self, spots: &[Spot], at: (f64, f64)) -> f64 {
        let [from, to] = spots else {
            return f64::INFINITY;
        };
        levels(from, to)
            .map(|level| segment_distance(at, level.left, level.right))
            .fold(segment_distance(at, from.place(), to.place()), f64::min)
    }
}
