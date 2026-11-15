//! The trend line: a line between two points.

use super::shape::{Brush, Shape, Spot, segment_distance};

pub(super) struct Trend;

impl Shape for Trend {
    fn draw(&self, brush: &mut Brush<'_>, spots: &[Spot]) {
        if let [from, to] = spots {
            brush.line(from.place(), to.place(), brush.ink.line);
        }
    }

    fn distance(&self, spots: &[Spot], at: (f64, f64)) -> f64 {
        match spots {
            [from, to] => segment_distance(at, from.place(), to.place()),
            _ => f64::INFINITY,
        }
    }
}
