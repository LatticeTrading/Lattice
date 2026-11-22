//! The position: a trade planned on the chart, with what it risks for what
//! it may make. Its first point is the entry (a price, and the time the
//! plan starts at), its second the target (a price, and the time it ends
//! at) and its third the stop, of which only the price is read. A target
//! over the entry plans a long, one under it a short.
//!
//! Between the entry and the target is the reward's zone, in the buy
//! color, and between the entry and the stop the risk's, in the sell
//! color, a line along each price. The target and the stop say their price
//! and what the position makes or loses there, as a share of the entry;
//! the entry says the side, its price and the reward over the risk
//! (`R:R 2.00`), once it has a stop that loses.
//!
//! While it is being made it is the reward's zone alone, until the target
//! is placed. It is grabbed by its edges, as a rectangle is, and its
//! stop's handle is at the target's end of the stop's line, whatever time
//! the stop was placed at ([`Shape::grips`]).

use super::shape::{Brush, Shape, Spot, segment_distance};
use crate::{format::price_text, widgets::common::width_of};

/// The names' text size, how far the target's and the stop's are written
/// outside their lines and in from their left ends, and how far the
/// entry's is from the plan's right end.
const TEXT: f64 = 10.0;
const NAME_AWAY: f64 = 7.0;
const NAME_IN: f64 = 2.0;
const NAME_BESIDE: f64 = 4.0;

/// How far down the plot the line through `at` is: on a whole point, so it
/// is one sharp row.
fn row(at: &Spot) -> f64 {
    at.y.floor() + 0.5
}

/// A position's points as they are placed: fewer than three while it is
/// being made.
struct Plan<'a> {
    entry: &'a Spot,
    target: &'a Spot,
    stop: Option<&'a Spot>,
}

impl<'a> Plan<'a> {
    fn of(spots: &'a [Spot]) -> Option<Self> {
        let (entry, target, stop) = match spots {
            [entry, target] => (entry, target, None),
            [entry, target, stop] => (entry, target, Some(stop)),
            _ => return None,
        };
        Some(Self {
            entry,
            target,
            stop,
        })
    }

    /// Its left and right ends: the entry's time and the target's.
    fn ends(&self) -> (f64, f64) {
        (
            self.entry.x.min(self.target.x),
            self.entry.x.max(self.target.x),
        )
    }

    /// Whether it buys: its target is no lower than its entry.
    fn long(&self) -> bool {
        self.target.anchor.price() >= self.entry.anchor.price()
    }

    /// What a unit of the position makes at `price`: less than nothing
    /// where it loses.
    fn made(&self, price: f64) -> f64 {
        let moved = price - self.entry.anchor.price();
        if self.long() { moved } else { -moved }
    }

    /// What the line at `price` says: `name`, the price, and what the
    /// position makes there as a share of its entry (none for an entry at
    /// no price).
    fn priced(&self, name: &str, price: f64) -> String {
        let (entry, made) = (self.entry.anchor.price(), self.made(price));
        if entry == 0.0 {
            return format!("{name} {}", price_text(price));
        }
        let sign = if made < 0.0 { "-" } else { "+" };
        let share = (made / entry).abs() * 100.0;
        format!("{name} {} ({sign}{share:.2}%)", price_text(price))
    }

    /// What the entry says: the side, its price, and the reward over the
    /// risk once the stop loses.
    fn entered(&self) -> String {
        let side = if self.long() { "Long" } else { "Short" };
        let entered = format!("{side} {}", price_text(self.entry.anchor.price()));
        let reward = self.made(self.target.anchor.price());
        let risk = self.stop.map(|stop| -self.made(stop.anchor.price()));
        match risk {
            Some(risk) if risk > 0.0 => format!("{entered}  R:R {:.2}", reward / risk),
            _ => entered,
        }
    }

    /// Its lines, each from its left end to its right: the entry's, the
    /// target's and the stop's, then its two sides.
    fn edges(&self) -> Vec<((f64, f64), (f64, f64))> {
        let (left, right) = self.ends();
        let stop = self.stop.map(row);
        let rows = [Some(row(self.entry)), Some(row(self.target)), stop];
        let rows: Vec<f64> = rows.into_iter().flatten().collect();
        let top = rows.iter().copied().fold(f64::INFINITY, f64::min);
        let bottom = rows.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let across = rows.iter().map(|y| ((left, *y), (right, *y)));
        let sides = [left, right].map(|x| ((x, top), (x, bottom)));
        across.chain(sides).collect()
    }
}

pub(super) struct Position;

impl Shape for Position {
    fn draw(&self, brush: &mut Brush<'_>, spots: &[Spot]) {
        let Some(plan) = Plan::of(spots) else {
            return;
        };
        let ink = brush.ink;
        let (left, right) = plan.ends();
        let (entry, target) = (row(plan.entry), row(plan.target));
        let stop = plan.stop.map(|stop| (stop, row(stop)));

        // The zones, then a line along each price, the entry's last.
        let width = right - left;
        brush.fill(
            left,
            entry.min(target),
            width,
            (target - entry).abs(),
            ink.up_tint,
        );
        if let Some((_, stop)) = stop {
            brush.fill(
                left,
                entry.min(stop),
                width,
                (stop - entry).abs(),
                ink.down_tint,
            );
        }
        brush.line((left, target), (right, target), ink.up);
        if let Some((_, stop)) = stop {
            brush.line((left, stop), (right, stop), ink.down);
        }
        brush.line((left, entry), (right, entry), ink.line);

        // The target's name and the stop's outside their lines, away from
        // the entry; the entry's beside its line, past the plan's right
        // end while that is on the plot.
        let outside = |edge: f64, over: bool| {
            if over {
                edge - NAME_AWAY
            } else {
                edge + NAME_AWAY
            }
        };
        let said = plan.priced("Target", plan.target.anchor.price());
        let middle = outside(target, target <= entry);
        brush.text(&said, left + NAME_IN, middle, ink.up);
        if let Some((spot, stop)) = stop {
            let said = plan.priced("Stop", spot.anchor.price());
            let middle = outside(stop, stop < entry);
            brush.text(&said, left + NAME_IN, middle, ink.down);
        }
        let said = plan.entered();
        let room = brush.width - width_of(&said, TEXT) - NAME_BESIDE;
        let x = (right + NAME_BESIDE).min(room).max(left + NAME_IN);
        brush.text(&said, x, entry, ink.text);
    }

    fn distance(&self, spots: &[Spot], at: (f64, f64)) -> f64 {
        let Some(plan) = Plan::of(spots) else {
            return f64::INFINITY;
        };
        plan.edges()
            .into_iter()
            .map(|(start, end)| segment_distance(at, start, end))
            .fold(f64::INFINITY, f64::min)
    }

    /// The stop's handle is at the target's end of its line: its own time
    /// says nothing.
    fn grips(&self, spots: &[Spot]) -> Vec<(f64, f64)> {
        match spots {
            [entry, target, stop] => vec![entry.place(), target.place(), (target.x, stop.y)],
            _ => spots.iter().map(Spot::place).collect(),
        }
    }
}
