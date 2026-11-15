//! What a kind of drawing looks like and where it can be grabbed
//! ([`Shape`]), and what the kinds draw with: their points as they are on
//! the plot ([`Spot`]), the theme's colors ([`Ink`]) and a pen kept to the
//! plot ([`Brush`]).
//!
//! A drawing may reach far off the plot (a line to a point months back,
//! seen zoomed in), so a [`Brush`] draws only the part of a line that is
//! on it: nothing is asked of the canvas beyond the plot, however far the
//! points are.

use nsh_core::prelude::*;

use super::Anchor;
use crate::{theme, widgets::common::Pen};

/// How far past the plot's edges a line is drawn, so its end never shows.
const MARGIN: f64 = 8.0;

/// A drawing's point as it is on the plot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Spot {
    /// Where it is, in points from the plot's top left.
    pub x: f64,
    pub y: f64,
    /// The slot of the series it is in.
    pub slot: f64,
    pub anchor: Anchor,
}

impl Spot {
    pub fn place(&self) -> (f64, f64) {
        (self.x, self.y)
    }
}

/// What a kind of drawing looks like and where it can be grabbed. Its
/// points come as `spots`, fewer than the kind takes while one is being
/// made.
pub(crate) trait Shape {
    fn draw(&self, brush: &mut Brush<'_>, spots: &[Spot]);

    /// How far `at` is from what of it can be grabbed (its lines), in
    /// points.
    fn distance(&self, spots: &[Spot], at: (f64, f64)) -> f64;
}

/// The colors drawings are made in.
pub(crate) struct Ink {
    /// A drawing's lines.
    pub line: Hsla,
    /// Its quieter lines: what joins the two ends of a retracement.
    pub dim: Hsla,
    pub text: Hsla,
    /// The box a readout is written in, and its edge.
    pub panel: Hsla,
    pub edge: Hsla,
    /// What a handle is filled with.
    pub ground: Hsla,
    /// A measurement up and one down: its lines, and its box's tint.
    pub up: Hsla,
    pub down: Hsla,
    pub up_tint: Hsla,
    pub down_tint: Hsla,
}

impl Ink {
    /// The colors in the theme now (tracked: what reads them runs again
    /// when the theme changes).
    pub fn themed() -> Self {
        let of = |role: theme::Role| color(role.get());
        Self {
            line: of(theme::ACCENT),
            dim: of(theme::TEXT_FAINT),
            text: of(theme::TEXT),
            panel: of(theme::PANEL.opacity(0.92)),
            edge: of(theme::BORDER),
            ground: of(theme::BACKGROUND),
            up: of(theme::BID),
            down: of(theme::ASK),
            up_tint: of(theme::BID.opacity(0.14)),
            down_tint: of(theme::ASK.opacity(0.14)),
        }
    }
}

/// What a shape draws with: a pen kept to the plot, the colors, and how
/// heavy its lines are.
pub(crate) struct Brush<'a> {
    pub pen: &'a mut Pen,
    pub ink: &'a Ink,
    /// The plot's size, in points.
    pub width: f64,
    pub height: f64,
    /// How wide a line is: wider while the drawing is under the pointer or
    /// picked.
    pub weight: f64,
}

impl Brush<'_> {
    /// A line from `from` to `to` in `color`: the part of it on the plot.
    pub fn line(&mut self, from: (f64, f64), to: (f64, f64), color: Hsla) {
        self.stroke(from, to, color, None);
    }

    /// A dashed line from `from` to `to` in `color`: the part of it on the
    /// plot, its dashes where they would be were all of it drawn.
    pub fn dashes(&mut self, from: (f64, f64), to: (f64, f64), color: Hsla) {
        self.stroke(from, to, color, Some((5.0, 4.0)));
    }

    fn stroke(&mut self, from: (f64, f64), to: (f64, f64), color: Hsla, dash: Option<(f32, f32)>) {
        let Some(cut) = clipped(from, to, self.width, self.height) else {
            return;
        };
        let weight = self.weight as f32;
        self.pen.save();
        self.pen.with(|painter| {
            let painter = painter.stroke_color(color).line_width(weight);
            match dash {
                Some((on, off)) => painter
                    .line_dash(&[px(on), px(off)])
                    .line_dash_offset((cut.before % f64::from(on + off)) as f32),
                None => painter,
            }
        });
        self.pen.line(cut.from.0, cut.from.1, cut.to.0, cut.to.1);
        self.pen.restore();
    }

    /// The rectangle from (`x`, `y`), `width` by `height`, filled with
    /// `color`: the part of it on the plot.
    pub fn fill(&mut self, x: f64, y: f64, width: f64, height: f64, color: Hsla) {
        let (left, top) = (x.max(0.0), y.max(0.0));
        let (right, bottom) = ((x + width).min(self.width), (y + height).min(self.height));
        if right > left && bottom > top {
            self.pen.fill(color);
            self.pen.rect(left, top, right - left, bottom - top);
        }
    }

    /// `text` in `color` from `x`, its middle at `y`.
    pub fn text(&mut self, text: &str, x: f64, y: f64, color: Hsla) {
        self.pen.fill(color);
        self.pen.text(text, x, y);
    }
}

/// The part of a line that is on the plot.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cut {
    from: (f64, f64),
    to: (f64, f64),
    /// How much of the line comes before it, in points.
    before: f64,
}

/// The part of the line from `from` to `to` that is on a plot of `width`
/// by `height` points (and [`MARGIN`] past its edges); none where no part
/// is.
fn clipped(from: (f64, f64), to: (f64, f64), width: f64, height: f64) -> Option<Cut> {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    if !(from.0.is_finite() && from.1.is_finite() && dx.is_finite() && dy.is_finite()) {
        return None;
    }
    // How far along the line (0 at `from`, 1 at `to`) it enters and leaves.
    let (mut enters, mut leaves) = (0.0_f64, 1.0_f64);
    for (toward, room) in [
        (-dx, from.0 + MARGIN),
        (dx, width + MARGIN - from.0),
        (-dy, from.1 + MARGIN),
        (dy, height + MARGIN - from.1),
    ] {
        if toward == 0.0 {
            if room < 0.0 {
                return None;
            }
        } else if toward < 0.0 {
            enters = enters.max(room / toward);
        } else {
            leaves = leaves.min(room / toward);
        }
        if enters > leaves {
            return None;
        }
    }
    let along = |share: f64| (from.0 + dx * share, from.1 + dy * share);
    Some(Cut {
        from: along(enters),
        to: along(leaves),
        before: enters * dx.hypot(dy),
    })
}

/// How far `at` is from the line from `from` to `to`.
pub(crate) fn segment_distance(at: (f64, f64), from: (f64, f64), to: (f64, f64)) -> f64 {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let length = dx * dx + dy * dy;
    let share = if length > 0.0 {
        (((at.0 - from.0) * dx + (at.1 - from.1) * dy) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (at.0 - from.0 - dx * share).hypot(at.1 - from.1 - dy * share)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(left: (f64, f64), right: (f64, f64)) -> bool {
        (left.0 - right.0).abs() < 1e-9 && (left.1 - right.1).abs() < 1e-9
    }

    #[test]
    fn a_line_is_cut_to_the_plot_and_keeps_its_dashes() {
        // Across a 100 by 50 plot from far to its left: what is on it, and
        // the 992 points before that.
        let cut = clipped((-1_000.0, 25.0), (50.0, 25.0), 100.0, 50.0).unwrap();
        assert!(near(cut.from, (-8.0, 25.0)) && near(cut.to, (50.0, 25.0)));
        assert!((cut.before - 992.0).abs() < 1e-9);
        // One on the plot is itself; one that misses it is nothing.
        let whole = clipped((10.0, 10.0), (90.0, 40.0), 100.0, 50.0).unwrap();
        let all = Cut {
            from: (10.0, 10.0),
            to: (90.0, 40.0),
            before: 0.0,
        };
        assert_eq!(whole, all);
        assert!(clipped((-50.0, -500.0), (500.0, -50.0), 100.0, 50.0).is_none());
        assert!(clipped((10.0, 80.0), (90.0, 80.0), 100.0, 50.0).is_none());
        assert!(clipped((f64::NAN, 0.0), (1.0, 1.0), 100.0, 50.0).is_none());
        // A slanted one keeps its slope where it is cut.
        let cut = clipped((-108.0, -108.0), (1_000.0, 1_000.0), 100.0, 50.0).unwrap();
        assert!(near(cut.from, (-8.0, -8.0)) && near(cut.to, (58.0, 58.0)));
    }

    #[test]
    fn the_distance_to_a_line_stops_at_its_ends() {
        let (from, to) = ((0.0, 0.0), (10.0, 0.0));
        assert_eq!(segment_distance((5.0, 3.0), from, to), 3.0);
        assert_eq!(segment_distance((13.0, 4.0), from, to), 5.0);
        assert_eq!(segment_distance((3.0, 4.0), from, from), 5.0);
    }
}
