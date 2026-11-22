//! The Trade Panel: places and cancels orders for the connected wallet's
//! account on an exchange it has enabled (the old app's `tradepanel`, which
//! was a ticket with nothing behind it).
//!
//! Data (the widget's session, [`TradeSession`] in [`session`]):
//!
//! - Whose orders: the account of the wallet connected in the Everything
//!   Bar ([`Wallet`](crate::data::Wallet)), on the exchange its settings
//!   name, once that account has enabled trading there
//!   ([`Trading`](crate::data::Trading)). With no wallet, or trading not
//!   enabled, the panel shows the rows that do both, the wallet panel's
//!   own. Trading enabled in an earlier run is taken up again when the
//!   account connects, so connecting is then all there is to do.
//! - What it reads while an account is connected: the account's perpetuals
//!   account (the portfolio's request, shared with every widget that
//!   watches the address), its open orders, what the exchange trades, and,
//!   only while the widget shows, the market's price, followed live where
//!   something prices the exchange ([`Prices`](crate::data::Prices)).
//! - While it trades for an account it follows that account live
//!   ([`Trading::follow`](crate::data::Trading::follow)): the exchange's
//!   stream brings its open orders and its figures as the market changes
//!   them (an order filled, one cancelled somewhere else), and each fill is
//!   said under the ticket as it happens. The requests above go on being
//!   polled underneath, and are all there is while the stream is down.
//! - How the account trades the market is shown and set beside it: its
//!   leverage and whether a position is backed by the whole account's
//!   margin or its own, set together.
//! - An order may carry a take profit and a stop loss, which wait for it
//!   to trade and close what it opened; the position the account already
//!   holds in the market shows under the ticket, with the exits to place
//!   for the whole of it.
//! - The order being written (its ticket) lives in the session, so another
//!   tab or page leaves it as it was. Sending it and cancelling an order
//!   are asked of trading, which asks the exchange; what the exchange made
//!   of it is said under the ticket, and the account and its open orders
//!   are asked for again at once.
//!
//! Settings ([`TradePanelSettings`]: the exchange, the market, the link
//! color, whether orders are confirmed) are saved in its tab record. It is
//! linkable: a market picked in the Everything Bar's search, on any
//! exchange, sets the asset traded.
//!
//! It knows no exchange: what it shows and sends is trading's
//! ([`crate::data::trading`]) and the portfolio's.

mod panel;
mod session;
mod settings;
#[cfg(test)]
mod tests;
mod view;

use std::rc::Rc;

use nsh_core::prelude::*;

use self::{session::TradeSession, settings::TradePanelSettings};
use crate::{
    data::{DataServices, SessionScope},
    widgets::{Category, Linking, TabContext, host::Widget},
};

/// The Trade Panel, as the registry lists it.
pub(crate) struct TradePanel;

impl Widget for TradePanel {
    const ID: &'static str = "tradepanel";
    const TITLE: &'static str = "Trade Panel";
    const DESCRIPTION: &'static str =
        "Place and cancel orders for your connected wallet's account.";
    const CATEGORY: Category = Category::Trading;

    type Settings = TradePanelSettings;
    type Session = TradeSession;

    fn session(
        scope: SessionScope,
        services: &DataServices,
        settings: Signal<TradePanelSettings>,
    ) -> TradeSession {
        TradeSession::new(
            scope,
            services.trading.clone(),
            services.prices.clone(),
            services.wallet.clone(),
            settings,
        )
    }

    /// Its ticket, for the market its tab's settings name.
    fn view(session: &Rc<TradeSession>, tab: &TabContext<TradePanelSettings>) -> View {
        view::trade_view(session, tab)
    }

    fn panel(session: &Rc<TradeSession>, tab: &TabContext<TradePanelSettings>) -> Option<Overlay> {
        Some(panel::panel(session, tab))
    }

    const LINKING: Option<Linking<TradePanelSettings>> = Some(Linking {
        link: TradePanelSettings::link,
        with_link: TradePanelSettings::with_link,
        with_market: TradePanelSettings::with_linked_market,
    });
}
