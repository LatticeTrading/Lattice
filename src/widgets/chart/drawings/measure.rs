//! The measurement: how far its second point is from its first. A tinted
//! box between the two (in the buy color for a higher price, the sell
//! color for a lower one), an arrow across it and one down it, and a
//! readout beside it: the price's change and its share of the price
//! measured from, then the bars and the time between the points.

use super::shape::{Brush, Shape, Spot};
use crate::{format::price_places, widgets::common::width_of};

/// The readout's text size, its lines' height, and its box's padding.
const TEXT: f64 = 10.0;
const LINE: f64 = 14.0;
const PADDING: f64 = 6.0;
/// How far the readout is from the box it is about.
const GAP: f64 = 6.0;
/// An arrow's head: how far back along it, and to each side.
const HEAD: f64 = 4.0;
const HEAD_HALF: f64 = 3.0;

/// What a measurement from `from` to `to` says, on two lines: the price's
/// change, in the decimals the prices are written in, with its share of
/// the price measured from; then the bars and the time between the two.
pub(crate) fn readout(from: &Spot, to: &Spot) -> [String; 2] {
    let (start, end) = (from.anchor.price(), to.anchor.price());
    let change = end - start;
    let sign = if change < 0.0 { "-" } else { "+" };
    let places = price_places(start.abs().max(end.abs()));
    let price = if start == 0.0 {
        format!("{sign}{:.places$}", change.abs())
    } else {
        format!(
            "{sign}{:.places$} ({sign}{:.2}%)",
            change.abs(),
            (change / start).abs() * 100.0
        )
    };
    let bars = (to.slot - from.slot).abs().round() as u64;
    let name = if bars == 1 { "bar" } else { "bars" };
    let time = lasting((to.anchor.time() - from.anchor.time()).abs());
    [price, format!("{bars} {name}, {time}")]
}

/// `seconds` in its two largest units: `45s`, `12m 30s`, `3h 20m`, `2d 4h`.
fn lasting(seconds: f64) -> String {
    let total = seconds.round() as u64;
    let (days, hours) = (total / 86_400, total / 3_600 % 24);
    let (minutes, seconds) = (total / 60 % 60, total % 60);
    let two = |large: u64, large_unit: &str, small: u64, small_unit: &str| match small {
        0 => format!("{large}{large_unit}"),
        _ => format!("{large}{large_unit} {small}{small_unit}"),
    };
    match (days, hours, minutes) {
        (0, 0, 0) => format!("{seconds}s"),
        (0, 0, _) => two(minutes, "m", seconds, "s"),
        (0, _, _) => two(hours, "h", minutes, "m"),
        _ => two(days, "d", hours, "h"),
    }
}

pub(super) struct Measure;

impl Shape for Measure {
    fn draw(&self, brush: &mut Brush<'_>, spots: &[Spot]) {
        let [from, to] = spots else {
            return;
        };
        let rising = to.anchor.price() >= from.anchor.price();
        let (line, tint) = if rising {
            (brush.ink.up, brush.ink.up_tint)
        } else {
            (brush.ink.down, brush.ink.down_tint)
        };
        let (left, right) = (from.x.min(to.x), from.x.max(to.x));
        let (top, bottom) = (from.y.min(to.y), from.y.max(to.y));
        brush.fill(left, top, right - left, bottom - top, tint);

        // An arrow across the box's middle and one down it, each from the
        // first point's side to the second's.
        let (across, down) = ((top + bottom) / 2.0, (left + right) / 2.0);
        brush.line((from.x, across), (to.x, across), line);
        brush.line((down, from.y), (down, to.y), line);
        let ahead = (to.x - from.x).signum() * HEAD;
        if (to.x - from.x).abs() > HEAD * 2.0 {
            for side in [-HEAD_HALF, HEAD_HALF] {
                brush.line((to.x, across), (to.x - ahead, across + side), line);
            }
        }
        let ahead = (to.y - from.y).signum() * HEAD;
        if (to.y - from.y).abs() > HEAD * 2.0 {
            for side in [-HEAD_HALF, HEAD_HALF] {
                brush.line((down, to.y), (down + side, to.y - ahead), line);
            }
        }

        // The readout, past the second point's end of the box: under it
        // for a lower price, over it for a higher one, kept on the plot.
        let lines = readout(from, to);
        let widest = lines.iter().map(|line| width_of(line, TEXT));
        let width = widest.fold(0.0, f64::max) + PADDING * 2.0;
        let height = LINE * lines.len() as f64 + PADDING;
        let x = (down - width / 2.0).clamp(0.0, (brush.width - width).max(0.0));
        let y = if to.y >= from.y {
            bottom + GAP
        } else {
            top - GAP - height
        };
        let y = y.clamp(0.0, (brush.height - height).max(0.0));
        brush.fill(x, y, width, height, brush.ink.panel);
        brush.pen.stroke(brush.ink.edge);
        brush
            .pen
            .outline(x + 0.5, y + 0.5, width - 1.0, height - 1.0);
        for (row, text) in lines.iter().enumerate() {
            let middle = y + PADDING / 2.0 + LINE * (row as f64 + 0.5);
            brush.text(text, x + PADDING, middle, brush.ink.text);
        }
    }

    /// Nothing of a measurement is grabbed: it is not kept.
    fn distance(&self, _: &[Spot], _: (f64, f64)) -> f64 {
        f64::INFINITY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_of_time_read_in_their_two_largest_units() {
        for (seconds, text) in [
            (0.0, "0s"),
            (45.4, "45s"),
            (720.0, "12m"),
            (750.0, "12m 30s"),
            (12_000.0, "3h 20m"),
            (86_400.0, "1d"),
            (187_200.0, "2d 4h"),
        ] {
            assert_eq!(lasting(seconds), text);
        }
    }
}
