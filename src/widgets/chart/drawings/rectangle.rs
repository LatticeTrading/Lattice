//! The rectangle: a zone between two prices over a span of time (a range,
//! a block of supply or demand), its two points opposite corners. It is
//! tinted, with a line along each edge.
//!
//! It is grabbed by an edge: a press inside it is the plot's, so a zone
//! over much of the plot does not keep the plot from being panned.

use super::shape::{Brush, Shape, Spot, segment_distance};

/// The edges of the rectangle with corners at `from` and `to`: its top,
/// its right, its bottom and its left.
fn edges(from: &Spot, to: &Spot) -> [((f64, f64), (f64, f64)); 4] {
    let (left, right) = (from.x.min(to.x), from.x.max(to.x));
    let (top, bottom) = (from.y.min(to.y), from.y.max(to.y));
    [
        ((left, top), (right, top)),
        ((right, top), (right, bottom)),
        ((right, bottom), (left, bottom)),
        ((left, bottom), (left, top)),
    ]
}

pub(super) struct Rectangle;

impl Shape for Rectangle {
    fn draw(&self, brush: &mut Brush<'_>, spots: &[Spot]) {
        let [from, to] = spots else {
            return;
        };
        let edges = edges(from, to);
        let ((left, top), (right, bottom)) = (edges[0].0, edges[2].0);
        brush.fill(left, top, right - left, bottom - top, brush.ink.tint);
        for (start, end) in edges {
            brush.line(start, end, brush.ink.line);
        }
    }

    fn distance(&self, spots: &[Spot], at: (f64, f64)) -> f64 {
        let [from, to] = spots else {
            return f64::INFINITY;
        };
        edges(from, to)
            .into_iter()
            .map(|(start, end)| segment_distance(at, start, end))
            .fold(f64::INFINITY, f64::min)
    }
}
