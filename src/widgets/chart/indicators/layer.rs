//! The indicators' layer: one canvas over the plot's area, under the
//! chart's drawings and the crosshair, on which every indicator that
//! shows is painted ([`paint`]): each of its lines through its value at
//! every bucket of the scene, over the bucket's middle, as the plot draws
//! a line's points, in its indicator's color or, for a trend's line, in
//! the bars' (the rising ones' under them, the falling ones' over them).
//! It is painted again when the plot moves or its series or an indicator
//! changes, and not at all while no indicator shows.

use std::rc::Rc;

use nsh_core::{canvas::LineJoin, prelude::*};

use super::Computed;
use crate::{
    link::LinkColor,
    theme,
    widgets::{
        chart::session::ChartSession,
        common::{
            Pen, Shared,
            plot::{self, Scene},
        },
    },
};

/// How wide a line is, in points.
const WIDTH: f32 = 1.25;
/// How opaque a quiet line is (a band about the main one).
const QUIET: f32 = 0.55;
/// How far off the plot (in plot heights) a value is drawn at most, as
/// the plot's own prices are: nothing is asked of the canvas beyond it.
const FAR: f64 = 64.0;

/// The colors lines are painted in: each link color's
/// ([`LinkColor::index`]), as it is and as a quiet line has it, and the
/// two a trend's line has, under the bars and over them.
pub(crate) struct Ink {
    pub plain: [Hsla; 5],
    pub quiet: [Hsla; 5],
    pub rising: Hsla,
    pub falling: Hsla,
}

impl Ink {
    /// The colors in the theme now (tracked: what reads them runs again
    /// when the theme changes).
    pub fn themed() -> Self {
        Self {
            plain: LinkColor::ALL.map(|link| color(link.role().get())),
            quiet: LinkColor::ALL.map(|link| color(link.role().opacity(QUIET).get())),
            rising: color(theme::BID.get()),
            falling: color(theme::ASK.get()),
        }
    }

    fn of(&self, link: LinkColor, quiet: bool) -> Hsla {
        let colors = if quiet { &self.quiet } else { &self.plain };
        colors[link.index()]
    }
}

/// `computed`'s lines painted in order (later ones over earlier ones) on
/// the plot showing `scene`, in `ink`'s colors: a line is cut where it has
/// no value, and where it starts over. A trend's line is painted a side
/// at a time, each in its color.
pub(crate) fn paint(scene: &Scene, computed: &Computed, ink: &Ink) -> Painter {
    let (width, height) = (f64::from(scene.width), f64::from(scene.height));
    let (top, bottom) = (-FAR * height, (FAR + 1.0) * height);
    let buckets = scene.first_bucket..scene.first_bucket + scene.buckets.len();
    let mut pen = Pen(Painter::new());
    pen.save();
    pen.clip(0.0, 0.0, width, height);
    for (indicator, lines) in computed.each() {
        for line in lines {
            let strokes = if line.is_trend() {
                vec![(ink.rising, Some(true)), (ink.falling, Some(false))]
            } else {
                vec![(ink.of(indicator.color(), line.is_quiet()), None)]
            };
            for (stroke, side) in strokes {
                pen.with(|painter| {
                    let mut painter = painter
                        .stroke_color(stroke)
                        .line_width(WIDTH)
                        .line_join(LineJoin::Round)
                        .begin_path();
                    // The bar the line was last drawn to, which the next
                    // is joined to unless the line starts over between
                    // them.
                    let mut drawn = None;
                    for at in buckets.clone() {
                        let spot = scene
                            .bucket_end(at)
                            .and_then(|(x, bar)| Some((x, bar, line.at(bar)?)))
                            // Of a trend's line, the bars on this side.
                            .filter(|(_, bar, _)| side.is_none() || line.rising_at(*bar) == side);
                        let Some((x, bar, value)) = spot else {
                            drawn = None;
                            continue;
                        };
                        let (x, y) = (x as f32, scene.y_of(value).clamp(top, bottom) as f32);
                        let joined = drawn.is_some_and(|drawn| !line.restarts(drawn, bar));
                        painter = if joined {
                            painter.line_to(x, y)
                        } else {
                            painter.move_to(x, y)
                        };
                        drawn = Some(bar);
                    }
                    painter.stroke()
                });
            }
        }
    }
    pen.restore();
    pen.0
}

/// The canvas of `session`'s indicators, over the plot's area: a child of
/// what the chart lays over its plot.
pub(crate) fn layer(session: &Rc<ChartSession>) -> View {
    let (scene, lines) = (session.plot.scene, session.indicators);
    // Nothing while no indicator shows, so the plot's moves paint nothing
    // then.
    let painted = computed(move || {
        let lines = lines.get();
        (!lines.0.is_blank()).then(|| {
            let ink = Ink::themed();
            scene.with(|scene| Shared::new(paint(&scene.0, &lines.0, &ink)))
        })
    });
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
        .test_id("chart-indicator-lines"),
    )
}
