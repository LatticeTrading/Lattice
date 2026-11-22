//! A canvas pen ([`Pen`]): NSH's `Painter` driven in place, with the
//! operations the widgets' overlays use, in `f64` points as their frames
//! have them.
//!
//! Text is placed without measuring it: the app's font is 0.6 em wide a
//! character ([`ADVANCE`], [`width_of`]), as the chart's labels are
//! placed.

use std::f32::consts::FRAC_PI_2;

use nsh_core::{canvas::CanvasTextAlign, prelude::*};

/// A character's width over its text size in the app's font (IBM Plex
/// Mono).
const ADVANCE: f64 = 0.6;
/// The fonts of an overlay's rows and of its line labels (the old
/// `canvasFont(11)` and `canvasFont(10)`).
pub(crate) const FONT: &str = "11px IBM Plex Mono";
pub(crate) const SMALL_FONT: &str = "10px IBM Plex Mono";

/// How wide `text` is at `size` points.
pub(crate) fn width_of(text: &str, size: f64) -> f64 {
    text.chars().count() as f64 * size * ADVANCE
}

/// A painter driven in place.
pub(crate) struct Pen(pub Painter);

impl Pen {
    pub fn with(&mut self, change: impl FnOnce(Painter) -> Painter) {
        self.0 = change(std::mem::take(&mut self.0));
    }

    pub fn save(&mut self) {
        self.with(Painter::save);
    }

    pub fn restore(&mut self) {
        self.with(Painter::restore);
    }

    pub fn font(&mut self, font: &str) {
        self.with(|painter| painter.font(font));
    }

    pub fn fill(&mut self, color: Hsla) {
        self.with(|painter| painter.fill_color(color));
    }

    /// One point wide lines in `color`.
    pub fn stroke(&mut self, color: Hsla) {
        self.with(|painter| painter.stroke_color(color).line_width(1.0_f32));
    }

    pub fn align(&mut self, align: CanvasTextAlign) {
        self.with(|painter| painter.text_align(align));
    }

    pub fn clip(&mut self, x: f64, y: f64, width: f64, height: f64) {
        self.with(|painter| painter.begin_path().rect(area(x, y, width, height)).clip());
    }

    pub fn rect(&mut self, x: f64, y: f64, width: f64, height: f64) {
        self.with(|painter| painter.fill_rect(x as f32, y as f32, width as f32, height as f32));
    }

    /// The shape whose corners are `corners`, in order round it, filled.
    pub fn area(&mut self, corners: &[(f64, f64)]) {
        let Some(((x, y), rest)) = corners.split_first() else {
            return;
        };
        self.with(|painter| {
            let start = painter.begin_path().move_to(*x as f32, *y as f32);
            let round = rest.iter().fold(start, |painter, (x, y)| {
                painter.line_to(*x as f32, *y as f32)
            });
            round.close_path().fill()
        });
    }

    pub fn outline(&mut self, x: f64, y: f64, width: f64, height: f64) {
        self.with(|painter| painter.stroke_rect(area(x, y, width, height)));
    }

    pub fn line(&mut self, x1: f64, y1: f64, x2: f64, y2: f64) {
        self.with(|painter| {
            painter
                .begin_path()
                .move_to(x1 as f32, y1 as f32)
                .line_to(x2 as f32, y2 as f32)
                .stroke()
        });
    }

    pub fn text(&mut self, text: &str, x: f64, y: f64) {
        self.with(|painter| painter.fill_text(text, x as f32, y as f32));
    }

    /// `text` narrowed to fit `most` points.
    pub fn text_within(&mut self, text: &str, x: f64, y: f64, most: f64) {
        self.with(|painter| {
            painter.fill_text_max_width(text, x as f32, y as f32, px(most.max(1.0) as f32))
        });
    }

    /// Moves the origin to (`x`, `y`) and turns text to read upward.
    pub fn turn(&mut self, x: f64, y: f64) {
        self.with(|painter| {
            painter
                .translate(x as f32, y as f32)
                .rotate(Radians(-FRAC_PI_2))
        });
    }
}

fn area(x: f64, y: f64, width: f64, height: f64) -> Bounds<Points> {
    bounds(
        point(px(x as f32), px(y as f32)),
        size(px(width.max(0.0) as f32), px(height.max(0.0) as f32)),
    )
}
