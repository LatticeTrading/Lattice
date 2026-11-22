//! The parallel channel: a trend line between its first two points, and
//! another alongside it through its third (a trend's support and its
//! resistance). The two are as long as one another and as far apart, in
//! price, all the way along; between them the channel is tinted, and a
//! dashed line runs midway.
//!
//! While it is being made it is its first line alone, until that one's
//! second point is placed. It is grabbed by either line: a press between
//! them is the plot's, as a rectangle's inside is.

use super::shape::{Brush, Shape, Spot, segment_distance};

/// How far down the plot the channel's other line is from its first: as
/// far as `through` is from the first line at its own time.
fn drop(from: &Spot, to: &Spot, through: &Spot) -> f64 {
    let run = to.x - from.x;
    // A line straight up has no price at another time: the other is as
    // far down as `through` is from its start.
    let slope = if run.abs() > f64::EPSILON {
        (to.y - from.y) / run
    } else {
        0.0
    };
    through.y - (from.y + slope * (through.x - from.x))
}

/// `spot`'s place `down` points down the plot.
fn lowered(spot: &Spot, down: f64) -> (f64, f64) {
    (spot.x, spot.y + down)
}

pub(super) struct Channel;

impl Shape for Channel {
    fn draw(&self, brush: &mut Brush<'_>, spots: &[Spot]) {
        match spots {
            [from, to] => brush.line(from.place(), to.place(), brush.ink.line),
            [from, to, through] => {
                let down = drop(from, to, through);
                let (start, end) = (lowered(from, down), lowered(to, down));
                brush.area(&[from.place(), to.place(), end, start], brush.ink.tint);
                brush.dashes(
                    lowered(from, down / 2.0),
                    lowered(to, down / 2.0),
                    brush.ink.dim,
                );
                brush.line(from.place(), to.place(), brush.ink.line);
                brush.line(start, end, brush.ink.line);
            }
            _ => {}
        }
    }

    fn distance(&self, spots: &[Spot], at: (f64, f64)) -> f64 {
        match spots {
            [from, to] => segment_distance(at, from.place(), to.place()),
            [from, to, through] => {
                let down = drop(from, to, through);
                let other = segment_distance(at, lowered(from, down), lowered(to, down));
                segment_distance(at, from.place(), to.place()).min(other)
            }
            _ => f64::INFINITY,
        }
    }
}
