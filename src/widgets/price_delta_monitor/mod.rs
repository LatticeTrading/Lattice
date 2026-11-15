//! The Price Delta Monitor: one market's price, and how far each of the
//! markets compared with it is from it, as a difference and as a
//! percentage of the base's price (the old app's `PriceDeltaMonitor`).
//!
//! What it shows: each market's price is its mid price, read from the top
//! of its book ([`OrderBook::mid_price`](crate::data::ferris::OrderBook::mid_price)),
//! and a row's difference is its market's price less the base's.
//!
//! Data (the widget's session, [`Monitor`] in [`session`]):
//!
//! - It holds one lease per market on its book at one level a side
//!   ([`Ferris::order_book`](crate::data::ferris::Ferris::order_book)), so
//!   it shares each topic with whatever else reads it. The leases follow
//!   the settings: a market kept keeps its lease, a new one is leased, and
//!   one removed is let go. A hidden widget keeps its leases, so showing it
//!   again is instant.
//! - A market is leased as it is saved. The old widget loaded every
//!   exchange's catalog first, to find each market in it, and waited for
//!   all of them before it subscribed to any; here a market whose topic
//!   the server refuses reads `Unavailable`, and the rest are not held up.
//! - A book's change only marks the widget dirty. While it shows, the rows
//!   are made again at most once a frame, and only what changed is set: a
//!   hidden widget makes none.
//!
//! Settings ([`PriceDeltaSettings`]: the base and the markets compared
//! with it) are saved in its tab record; the session keeps a copy the host
//! brings up to date. The view edits them ([`view`]): the widget has no
//! settings panel, as the old one had none.

mod session;
mod settings;
#[cfg(test)]
mod tests;
mod view;

use std::rc::Rc;

use nsh_core::prelude::*;

use self::{session::Monitor, settings::PriceDeltaSettings};
use crate::{
    data::{DataServices, SessionScope},
    widgets::{Category, TabContext, host::Widget},
};

/// The Price Delta Monitor, as the registry lists it.
pub(crate) struct PriceDeltaMonitor;

impl Widget for PriceDeltaMonitor {
    const ID: &'static str = "price-delta-monitor";
    const TITLE: &'static str = "Price Delta Monitor";
    const DESCRIPTION: &'static str = "Monitor price differences between markets.";
    const CATEGORY: Category = Category::Trading;

    type Settings = PriceDeltaSettings;
    type Session = Monitor;

    fn session(
        scope: SessionScope,
        services: &DataServices,
        settings: Signal<PriceDeltaSettings>,
    ) -> Monitor {
        Monitor::new(scope, services.ferris.clone(), settings)
    }

    /// The base and its table, for the markets its tab's settings say.
    fn view(monitor: &Rc<Monitor>, tab: &TabContext<PriceDeltaSettings>) -> View {
        view::monitor_view(monitor, tab)
    }
}
