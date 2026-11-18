//! The Trade History: an account's latest trades, newest first, a row each
//! with when it was, what was traded and which way (and what it did to the
//! position), its price, size and worth, its fee and what it closed; over
//! them, what the trades listed come to (their volume, fees and closed
//! profit and loss).
//!
//! Data (the widget's session, [`TradeHistorySession`] in [`session`]):
//!
//! - It watches its account's trades
//!   ([`Portfolio::watch_fills`](crate::data::Portfolio::watch_fills)),
//!   which the account's exchange is asked for directly and every widget
//!   watching the account shares, every five minutes. The watch follows the
//!   settings: another account is watched before the last one is let go,
//!   and with none there is no watch. A hidden widget keeps its watch, so
//!   showing it again is instant.
//! - While it shows, each report makes the rows again: the latest
//!   [`session::MAX_ROWS`] (the totals count every trade reported). A
//!   hidden widget makes none.
//!
//! Settings ([`TradeHistorySettings`]: the account) are saved in its tab
//! record; the session keeps a copy the host brings up to date. The pane's
//! gear opens them, and so does the account's strip; with no account the
//! widget asks for one itself ([`view`]).
//!
//! It knows no exchange: what it shows is the portfolio's
//! ([`crate::data::portfolio`]), and what every account widget shares is
//! in [`crate::widgets::common::accounts`].

mod session;
mod settings;
#[cfg(test)]
mod tests;
mod view;

use std::rc::Rc;

use nsh_core::prelude::*;

use self::{session::TradeHistorySession, settings::TradeHistorySettings};
use crate::{
    data::{DataServices, SessionScope},
    widgets::{Category, TabContext, common::accounts::account_panel, host::Widget},
};

/// The Trade History, as the registry lists it.
pub(crate) struct TradeHistory;

impl Widget for TradeHistory {
    const ID: &'static str = "trade-history";
    const TITLE: &'static str = "Trade History";
    const DESCRIPTION: &'static str = "Review an account's latest trades, fees and closed PnL.";
    const CATEGORY: Category = Category::Portfolio;

    type Settings = TradeHistorySettings;
    type Session = TradeHistorySession;

    fn session(
        scope: SessionScope,
        services: &DataServices,
        settings: Signal<TradeHistorySettings>,
    ) -> TradeHistorySession {
        TradeHistorySession::new(scope, services.portfolio.clone(), settings)
    }

    /// Its totals and table, for the account its tab's settings name.
    fn view(session: &Rc<TradeHistorySession>, tab: &TabContext<TradeHistorySettings>) -> View {
        view::history_view(session, tab)
    }

    fn panel(
        session: &Rc<TradeHistorySession>,
        tab: &TabContext<TradeHistorySettings>,
    ) -> Option<Overlay> {
        Some(account_panel(
            tab,
            &session.portfolio,
            "Trade History settings",
            [],
        ))
    }
}
