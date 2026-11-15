//! The colors two books are drawn in (the old order book's): buy and sell
//! for the primary book, the accent and amber for the second pair.
//! [`Colors`] is what a canvas draws with, [`Palette`] what a shader does.

use nsh_core::{prelude::*, shader::UniformValue};

use super::{Pair, Side};
use crate::theme;

/// The colors a book widget's canvas draws with.
pub(crate) struct Colors {
    pub text: Hsla,
    pub dim: Hsla,
    /// Dividers and box edges.
    pub grid: Hsla,
    /// The price column and the pointer's box.
    pub panel: Hsla,
    pub bid: Hsla,
    pub ask: Hsla,
    /// The primary book's lines; the second pair's bids.
    pub accent: Hsla,
    /// The second pair's lines and asks.
    pub amber: Hsla,
}

impl Colors {
    /// The old overlay's colors in the theme now (tracked: what reads them
    /// runs again when the theme changes).
    pub fn themed() -> Self {
        let of = |role: theme::Role| color(role.get());
        Self {
            text: of(theme::TEXT),
            dim: of(theme::TEXT_MUTED),
            grid: of(theme::BORDER),
            panel: of(theme::PANEL.opacity(0.92)),
            bid: of(theme::BID),
            ask: of(theme::ASK),
            accent: of(theme::ACCENT),
            amber: of(theme::WARNING),
        }
    }

    /// The color of `pair`'s lines and name.
    pub fn line(&self, pair: Pair) -> Hsla {
        match pair {
            Pair::Primary => self.accent,
            Pair::Secondary => self.amber,
        }
    }

    /// The color of `pair`'s bids or asks.
    pub fn side(&self, pair: Pair, side: Side) -> Hsla {
        match (pair, side) {
            (Pair::Primary, Side::Bid) => self.bid,
            (Pair::Primary, Side::Ask) => self.ask,
            (Pair::Secondary, Side::Bid) => self.accent,
            (Pair::Secondary, Side::Ask) => self.amber,
        }
    }
}

/// The colors a book widget's shader draws depth with.
pub(crate) struct Palette {
    /// The primary book's bids and asks.
    pub bid: UniformValue,
    pub ask: UniformValue,
    /// The second pair's bids and asks.
    pub pair_bid: UniformValue,
    pub pair_ask: UniformValue,
}

impl Palette {
    /// The books' colors in the theme now, at `alpha` (tracked: what reads
    /// them runs again when the theme changes).
    pub fn themed(alpha: f32) -> Self {
        let tint = |role: theme::Role| {
            UniformValue::from(nsh_core::element::color(role.opacity(alpha).get()))
        };
        Self {
            bid: tint(theme::BID),
            ask: tint(theme::ASK),
            pair_bid: tint(theme::ACCENT),
            pair_ask: tint(theme::WARNING),
        }
    }

    /// The color of `pair`'s bids or asks.
    pub fn of(&self, pair: Pair, side: Side) -> UniformValue {
        match (pair, side) {
            (Pair::Primary, Side::Bid) => &self.bid,
            (Pair::Primary, Side::Ask) => &self.ask,
            (Pair::Secondary, Side::Bid) => &self.pair_bid,
            (Pair::Secondary, Side::Ask) => &self.pair_ask,
        }
        .clone()
    }
}
