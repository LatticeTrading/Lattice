//! The Depth Chart: one market's bids and asks as curves of running depth
//! either side of its mid price, with a second market on the same prices
//! if wanted, drawn on the GPU (the old app's `DepthChart` and
//! `VerticalDepthChart`, which drew one market on a 2D canvas). It is the
//! Order Book ([`crate::widgets::order_book`]) with every level summed
//! into the ones nearer the mid price: the same books, feeds, top bar and
//! settings rows (`crate::widgets::common::books`), the same split
//! between a shader for the depth ([`shader`]) and a canvas for what is
//! written over it ([`overlay`], which draws the depth too on a machine
//! without a GPU).
//!
//! Two markets are drawn together as the settings say: over one another,
//! each from the baseline; stacked, the second on top of the first, so
//! the outline is their sum; or combined into that sum alone.
//!
//! Data (the widget's session, [`DepthChartSession`] in [`session`]):
//!
//! - It holds one lease per book its settings draw on the market's shared
//!   book feed at the depth they ask for
//!   ([`Ferris::order_book`](crate::data::ferris::Ferris::order_book)): a
//!   depth chart and an order book of one market and depth share one topic
//!   and one snapshot. A hidden depth chart keeps streaming, so showing it
//!   again is instant.
//! - Feeds tell it of each change from the socket's callback; it only
//!   marks itself dirty there. While it shows, it makes the [`frame`] its
//!   view draws at most once a frame, and at once when the user zooms or
//!   the plot is resized. A hidden depth chart does none of it.
//! - The window of prices it looks at lives in the session
//!   ([`session::Window`]): about the books' mid price, as far as every
//!   side was loaded when the books arrived or as the user zoomed, a
//!   share of the price it keeps from frame to frame. It is fitted again
//!   when the books change, or have outgrown it.
//! - As the mid price moves, nothing on the plot does: the curves meet at
//!   its middle, the price marks are at places of their own, and only
//!   the prices they name change.
//!
//! Settings ([`DepthChartSettings`]: the two markets, the depth, the
//! grouping, how two books are drawn together, what depth is measured in,
//! the mid price's line and the layout) are saved in its tab record; the
//! top bar ([`view`]) changes the markets, the pairs' mode, the grouping
//! and the depth, and the pane's gear opens its settings panel
//! ([`panel`]) for them all.
//!
//! Its id is the old `depth-chart`. The old `vertical-depth-chart` is its
//! Layout setting here, as the old vertical order book is the Order
//! Book's.

mod frame;
mod overlay;
mod panel;
mod session;
mod settings;
mod shader;
#[cfg(test)]
mod tests;
mod view;

use std::rc::Rc;

use nsh_core::prelude::*;

use self::{session::DepthChartSession, settings::DepthChartSettings};
use crate::{
    data::{DataServices, SessionScope},
    widgets::{Category, TabContext, host::Widget},
};

/// The Depth Chart, as the registry lists it.
pub(crate) struct DepthChart;

impl Widget for DepthChart {
    const ID: &'static str = "depth-chart";
    const TITLE: &'static str = "Depth Chart";
    const DESCRIPTION: &'static str =
        "Compare cumulative bid and ask liquidity around the market price.";
    const CATEGORY: Category = Category::Charts;

    type Settings = DepthChartSettings;
    type Session = DepthChartSession;

    fn session(
        scope: SessionScope,
        services: &DataServices,
        settings: Signal<DepthChartSettings>,
    ) -> DepthChartSession {
        DepthChartSession::new(scope, services.ferris.clone(), settings)
    }

    /// Its plot, for the books its tab's settings draw.
    fn view(session: &Rc<DepthChartSession>, tab: &TabContext<DepthChartSettings>) -> View {
        view::depth_chart_view(session, tab)
    }

    fn panel(
        _session: &Rc<DepthChartSession>,
        tab: &TabContext<DepthChartSettings>,
    ) -> Option<Overlay> {
        Some(panel::panel(tab.clone()))
    }
}
