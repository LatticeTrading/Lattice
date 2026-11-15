//! What a book widget's overlay hands its view ([`Drawn`]): the canvas's
//! drawing, and the exchange logos that go over it. A canvas draws no SVG,
//! so the tiny logos beside its line labels are views, put where the
//! overlay says ([`logos`]). And what a plot says while it has no book
//! ([`status`]).

use nsh_core::prelude::*;

use crate::{exchanges, theme, widgets::common::Shared};

/// What a plot says in its middle while no book has arrived.
pub(crate) const WAITING: &str = "Waiting for order book...";
pub(crate) const UNAVAILABLE: &str = "Live data is unavailable for this market.";

/// An exchange's logo beside a line label: where the view puts it, in
/// points from the canvas's top left.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Logo {
    pub exchange: String,
    pub x: f32,
    pub y: f32,
    pub size: f32,
}

/// One overlay: the canvas's drawing, and the logos that go over it.
pub(crate) struct Drawn {
    pub painter: Painter,
    pub logos: Vec<Logo>,
}

/// What a plot says in its middle, from whether each of its books has
/// arrived (`loaded`) and whether its topic was refused (`failed`):
/// nothing once one has arrived.
pub(crate) fn status(loaded: &[bool], failed: &[bool]) -> &'static str {
    if loaded.iter().any(|loaded| *loaded) {
        ""
    } else if failed.iter().any(|failed| *failed) {
        UNAVAILABLE
    } else {
        WAITING
    }
}

/// The exchange logos beside an overlay's line labels, where it placed
/// them: `most` views over the canvas, each hidden while the overlay has
/// no logo for it.
pub(crate) fn logos(most: usize, drawn: Memo<Shared<Drawn>>) -> impl Iterator<Item = View> {
    (0..most).map(move |slot| {
        let logo = computed(move || drawn.with(|drawn| drawn.0.logos.get(slot).cloned()));
        let source = computed(move || logo.with(|logo| exchanges::logo(&logo.as_ref()?.exchange)));
        let of = move |read: fn(&Logo) -> f32| logo.with(|logo| logo.as_ref().map_or(0.0, read));
        view()
            .absolute()
            .left(move || px(of(|logo| logo.x)))
            .top(move || px(of(|logo| logo.y)))
            .size(move || px(of(|logo| logo.size)))
            .text_color(theme::TEXT_MUTED)
            .style_when(move || source.with(Option::is_none), |s| s.hidden())
            .child(svg(move || source.get().unwrap_or_else(exchanges::blank_logo)).size_full())
    })
}
