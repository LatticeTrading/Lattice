//! Double clicks among a touch responder's releases: NSH reports presses,
//! not how many, so the plots that take one (the chart's axes, the order
//! book's price column) count them here.

use std::{cell::Cell, rc::Rc};

use nsh_core::prelude::*;

/// How far a press may move and still be a click, in points.
pub(crate) const CLICK_SLOP: f32 = 3.0;
/// How long after a click another makes a double click.
const DOUBLE_CLICK_MS: f64 = 400.0;

/// Tells a double click among a responder's releases: two that barely
/// moved, soon after one another.
#[derive(Clone, Default)]
pub(crate) struct Clicks(Rc<Cell<Option<f64>>>);

impl Clicks {
    /// Whether the press `event` ends was the second click of a double
    /// click.
    pub fn doubled(&self, event: &ResponderEvent) -> bool {
        let moved = event
            .gesture
            .delta
            .x
            .0
            .abs()
            .max(event.gesture.delta.y.0.abs());
        if moved > CLICK_SLOP {
            self.0.set(None);
            return false;
        }
        let before = self.0.replace(Some(event.timestamp));
        let double = before.is_some_and(|before| event.timestamp - before <= DOUBLE_CLICK_MS);
        if double {
            self.0.set(None);
        }
        double
    }
}
