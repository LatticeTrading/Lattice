//! Where a drawing's points are on the plot, and which point a place of
//! the plot is ([`Scale`]): the times of the series' slots
//! ([`Timeline`]) with the slots and prices the plot shows ([`Scene`]).

use super::{Anchor, shape::Spot};
use crate::widgets::common::plot::{Scene, Timeline};

/// The plot's scale now: what turns points into places on it and back.
pub(crate) struct Scale<'a> {
    scene: &'a Scene,
    times: Timeline<'a>,
}

impl<'a> Scale<'a> {
    /// The scale of the plot showing `scene`, of the series whose slots'
    /// times are `times`.
    pub fn new(scene: &'a Scene, times: Timeline<'a>) -> Self {
        Self { scene, times }
    }

    /// The plot's size, in points.
    pub fn size(&self) -> (f64, f64) {
        (f64::from(self.scene.width), f64::from(self.scene.height))
    }

    /// Where `anchor` is on the plot, or off it; nowhere while the series
    /// has no bar.
    pub fn spot(&self, anchor: Anchor) -> Option<Spot> {
        let slot = self.times.slot_of(anchor.time())?;
        Some(Spot {
            x: self.scene.x_of(slot),
            y: self.scene.y_of(anchor.price()),
            slot,
            anchor,
        })
    }

    /// Where each of `points` is; nowhere while the series has no bar.
    pub fn spots(&self, points: &[Anchor]) -> Option<Vec<Spot>> {
        points.iter().map(|point| self.spot(*point)).collect()
    }

    /// The point (`x`, `y`) points from the plot's top left; none while
    /// the series has no bar.
    pub fn anchor(&self, x: f64, y: f64) -> Option<Anchor> {
        let time = self.times.time_at(self.scene.slot_at(x))?;
        Anchor::at(time, self.scene.price_at(y))
    }
}
