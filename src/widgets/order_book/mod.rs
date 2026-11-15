//! The Order Book: one market's bids and asks as depth bars along a price
//! ladder, with a second market beside it on the same ladder if wanted,
//! drawn on the GPU (the old app's `WebGLOrderBook` and its
//! `orderbook-gl` renderer, moved to WebGPU's shading language as the
//! Chart was; [`shader`] says how its bars are drawn, and [`overlay`]
//! how everything over them is, and the bars too on a machine without a
//! GPU).
//!
//! Data (the widget's session, [`OrderBookSession`] in [`session`]):
//!
//! - It holds one lease per book its settings draw on the market's shared
//!   book feed at the depth they ask for
//!   ([`Ferris::order_book`](crate::data::ferris::Ferris::order_book)):
//!   two order books of one market and depth share one topic and one
//!   snapshot. A book the settings drop is let go; one they keep keeps
//!   its feed. A hidden order book keeps streaming, so showing it again is
//!   instant.
//! - Feeds tell it of each change from the socket's callback; it only
//!   marks itself dirty there. While it shows, it makes the [`frame`] its
//!   view draws at most once a frame, and at once when the user moves the
//!   ladder or the plot is resized. A hidden order book does none of it.
//! - Where the ladder looks lives in the session
//!   ([`session::Ladder`]): on the mid price of the book the settings
//!   follow, or where the user left it; it starts over when the layout,
//!   the grouping, the row rule or the books change.
//!
//! Settings ([`OrderBookSettings`]: the two markets, the book followed,
//! the depth, the grouping, the layout and six switches) are saved in its
//! tab record under the old app's names; the top bar ([`view`]) changes
//! the markets, the book followed, the grouping and the depth, and the
//! pane's gear opens its settings panel ([`panel`]) for all but the book
//! followed.
//!
//! Its id stays the old `webgl-orderbook`, which saved tabs name; it is
//! the Order Book here, since no WebGL draws it on desktop.

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

use self::{session::OrderBookSession, settings::OrderBookSettings};
use crate::{
    data::{DataServices, SessionScope},
    widgets::{Category, TabContext, host::Widget},
};

/// The Order Book, as the registry lists it.
pub(crate) struct OrderBook;

impl Widget for OrderBook {
    const ID: &'static str = "webgl-orderbook";
    const TITLE: &'static str = "Order Book";
    const DESCRIPTION: &'static str =
        "Explore a GPU-rendered order book with optional paired markets.";
    const CATEGORY: Category = Category::OrderFlow;

    type Settings = OrderBookSettings;
    type Session = OrderBookSession;

    fn session(
        scope: SessionScope,
        services: &DataServices,
        settings: Signal<OrderBookSettings>,
    ) -> OrderBookSession {
        OrderBookSession::new(scope, services.ferris.clone(), settings)
    }

    /// Its ladder, for the books its tab's settings draw.
    fn view(session: &Rc<OrderBookSession>, tab: &TabContext<OrderBookSettings>) -> View {
        view::order_book_view(session, tab)
    }

    fn panel(
        _session: &Rc<OrderBookSession>,
        tab: &TabContext<OrderBookSettings>,
    ) -> Option<Overlay> {
        Some(panel::panel(tab.clone()))
    }
}
