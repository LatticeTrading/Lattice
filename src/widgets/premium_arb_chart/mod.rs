//! The Premium/Arb Chart: how far apart two markets' prices are, as a line
//! over time (the old app's `PremiumArbChart`): a perpetual against its
//! spot, one exchange's market against another's. It is the Chart's tick
//! line of a series of its own, drawn with the same GPU plot
//! ([`crate::widgets::common::plot`]: the scene, the shader, the viewport
//! and the view with its axes, crosshair and pointer handling), as the old
//! one drew with the old chart's engine.
//!
//! What it plots: each market's mid price is read from the top of its book
//! (halfway between the best bid and the best ask), and every change of
//! either makes a point of the second market's price less the primary's
//! ([`settings::Display`]: that difference, or it as a percentage of the
//! primary's price).
//!
//! Data (the widget's session, [`PremiumArbSession`] in [`session`]):
//!
//! - It holds one lease on its pair's prices
//!   ([`Ferris::price_pair`](crate::data::ferris::Ferris::price_pair)): the
//!   shared feed that reads both books and keeps the paired prices, the
//!   newest 12,000. Two charts of the same two markets in the same order
//!   share one history, whichever display mode each is in. A hidden chart
//!   keeps its lease, so the history goes on and showing it again is
//!   instant.
//! - The feed tells it of each sample from the socket's callback; it only
//!   marks its plot there. While it shows, it takes in the new samples as
//!   its display mode plots them and makes the scene its view draws, at
//!   most once a frame, and at once when the viewport moves or the plot is
//!   resized. A hidden chart does none of it.
//! - The viewport lives in the session, so a chart is where it was left
//!   when its page shows again. It starts over when a market changes;
//!   another display mode keeps where it looks and fits the prices again.
//! - With a market missing it holds no lease, and its view asks for the
//!   market instead ([`view`]).
//!
//! Settings ([`PremiumArbSettings`]: the two markets, the display mode and
//! the grid) are saved in its tab record; the pane's gear opens its
//! settings panel ([`panel`]).
//!
//! It cannot be linked, as the old one could not: a link color sends one
//! market, and it has two.

mod panel;
mod session;
mod settings;
#[cfg(test)]
mod tests;
mod view;

use std::rc::Rc;

use nsh_core::prelude::*;

use self::{session::PremiumArbSession, settings::PremiumArbSettings};
use crate::{
    data::{DataServices, SessionScope},
    widgets::{Category, TabContext, host::Widget},
};

/// The Premium/Arb Chart, as the registry lists it.
pub(crate) struct PremiumArbChart;

impl Widget for PremiumArbChart {
    const ID: &'static str = "premium-arb-chart";
    const TITLE: &'static str = "Premium/Arb Chart";
    const DESCRIPTION: &'static str = "Track the price premium between two exchange markets.";
    const CATEGORY: Category = Category::Charts;

    type Settings = PremiumArbSettings;
    type Session = PremiumArbSession;

    fn session(
        scope: SessionScope,
        services: &DataServices,
        settings: Signal<PremiumArbSettings>,
    ) -> PremiumArbSession {
        PremiumArbSession::new(scope, services.ferris.clone(), settings)
    }

    /// Its plot of the two markets its tab's settings compare, or its
    /// chooser while one is missing.
    fn view(session: &Rc<PremiumArbSession>, tab: &TabContext<PremiumArbSettings>) -> View {
        view::premium_arb_view(session, tab)
    }

    fn panel(
        _session: &Rc<PremiumArbSession>,
        tab: &TabContext<PremiumArbSettings>,
    ) -> Option<Overlay> {
        Some(panel::panel(tab.clone()))
    }
}
