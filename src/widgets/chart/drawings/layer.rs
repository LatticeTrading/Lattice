//! The drawings' layer: one canvas over the plot's area, under the
//! crosshair, on which every drawing that shows is painted ([`paint`]):
//! the saved ones of the market charted, the measurement last made and
//! the one being made. It is painted again when the plot moves, a bar
//! arrives or a drawing changes, and not at all while there is none.

use std::rc::Rc;

use nsh_core::{canvas::CanvasTextBaseline, prelude::*};

use super::{
    Anchor, Editor, Kind,
    scale::Scale,
    shape::{Brush, Ink, Spot},
};
use crate::widgets::common::{Pen, SMALL_FONT, Shared, plot};

/// A handle's side, in points: the square on each point of the drawing
/// that is picked or being made.
const HANDLE: f64 = 7.0;

/// How a drawing shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Emphasis {
    Plain,
    /// Under the pointer: its lines are wider.
    Hovered,
    /// Picked, or being made: wider lines, and a handle on each point.
    Picked,
}

/// One drawing to paint: its kind, its points (fewer than the kind takes
/// while it is being made) and how it shows.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Item {
    pub kind: Kind,
    pub points: Vec<Anchor>,
    pub emphasis: Emphasis,
}

/// `items` painted in order (later ones over earlier ones) on the plot
/// `scale` is of, in `ink`'s colors.
pub(crate) fn paint(scale: &Scale<'_>, items: &[Item], ink: &Ink) -> Painter {
    let (width, height) = scale.size();
    let mut pen = Pen(Painter::new());
    pen.save();
    pen.clip(0.0, 0.0, width, height);
    pen.font(SMALL_FONT);
    pen.with(|painter| painter.text_baseline(CanvasTextBaseline::Middle));
    for item in items {
        let Some(spots) = scale.spots(&item.points) else {
            continue;
        };
        let mut brush = Brush {
            pen: &mut pen,
            ink,
            width,
            height,
            weight: match item.emphasis {
                Emphasis::Plain => 1.0,
                Emphasis::Hovered | Emphasis::Picked => 2.0,
            },
        };
        item.kind.shape().draw(&mut brush, &spots);
        if item.emphasis == Emphasis::Picked {
            for spot in &spots {
                handle(&mut pen, spot, ink, width, height);
            }
        }
    }
    pen.restore();
    pen.0
}

/// The handle on `spot`, while it is on the plot.
fn handle(pen: &mut Pen, spot: &Spot, ink: &Ink, width: f64, height: f64) {
    let reach = HANDLE / 2.0;
    let on_plot =
        (-reach..=width + reach).contains(&spot.x) && (-reach..=height + reach).contains(&spot.y);
    if !on_plot {
        return;
    }
    // On whole points, so its edge is one sharp line.
    let (left, top) = ((spot.x - reach).round(), (spot.y - reach).round());
    pen.fill(ink.ground);
    pen.rect(left, top, HANDLE, HANDLE);
    pen.stroke(ink.line);
    pen.outline(left + 0.5, top + 0.5, HANDLE - 1.0, HANDLE - 1.0);
}

/// The canvas of `editor`'s drawings, over the plot's area: a child of
/// what the chart lays over its plot.
pub(crate) fn layer(editor: &Rc<Editor>) -> View {
    let editor = editor.clone();
    // Nothing while no drawing shows, so nothing is painted then.
    let painted = computed(move || editor.painted().map(Shared::new));
    plot::area().child(
        canvas(move || {
            painted.with(|painted| {
                painted
                    .as_ref()
                    .map_or_else(Painter::new, |painted| painted.0.as_ref().clone())
            })
        })
        .absolute()
        .size_full()
        .test_id("chart-drawings"),
    )
}
