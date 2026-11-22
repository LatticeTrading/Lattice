//! The horizontal line: a price marked across the whole plot (a support, a
//! resistance, a level to watch), whatever bars show. Its one point is
//! where it was placed, and where its handle is; its price is written over
//! its right end.

use super::shape::{Brush, Shape, Spot};
use crate::{format::price_text, widgets::common::width_of};

/// The price's text size, and how far it is written above the line (under
/// it at the plot's top, where it would not show) and in from its right
/// end.
const TEXT: f64 = 10.0;
const NAME_ABOVE: f64 = 7.0;
const NAME_IN: f64 = 4.0;

/// How far down the plot the line through `at` is: on a whole point, so it
/// is one sharp row.
fn row(at: &Spot) -> f64 {
    at.y.floor() + 0.5
}

pub(super) struct Horizontal;

impl Shape for Horizontal {
    fn draw(&self, brush: &mut Brush<'_>, spots: &[Spot]) {
        let [at] = spots else {
            return;
        };
        let y = row(at);
        brush.line((0.0, y), (brush.width, y), brush.ink.line);
        // Its price is written only while the line is on the plot.
        if !(0.0..=brush.height).contains(&y) {
            return;
        }
        let price = price_text(at.anchor.price());
        let x = (brush.width - width_of(&price, TEXT) - NAME_IN).max(0.0);
        let above = y - NAME_ABOVE;
        let middle = if above < TEXT / 2.0 {
            y + NAME_ABOVE
        } else {
            above
        };
        brush.text(&price, x, middle, brush.ink.text);
    }

    fn distance(&self, spots: &[Spot], at: (f64, f64)) -> f64 {
        match spots {
            [spot] => (at.1 - row(spot)).abs(),
            _ => f64::INFINITY,
        }
    }
}
