//! The Positions: an account's open positions, a row each with its size,
//! worth, entry and mark prices, unrealized profit and loss, liquidation
//! price and margin; or several accounts' in one table, on one exchange or
//! on several, each row marked with whose it is. The connected wallet's own
//! are reduced and closed from their rows.
//!
//! Data (the widget's session, [`PositionsSession`] in [`session`]):
//!
//! - It watches its account's margin report
//!   ([`Portfolio::watch_margin`](crate::data::Portfolio::watch_margin)),
//!   which the account's exchange is asked for directly and every widget
//!   watching the account shares. The watch follows the settings: another
//!   account is watched before the last one is let go, and with none there
//!   is no watch. A hidden widget keeps its watch, so showing it again is
//!   instant.
//! - Of several accounts it watches each one's report, and lists an
//!   account's positions as soon as its own arrives, none waiting for
//!   another: while some are still on their way the frame says how many
//!   show, and with some missing it says which are not.
//! - While it shows, each report makes the rows again: the positions worth
//!   the most first, whichever account each is of. A hidden widget makes
//!   none.
//! - While it shows, it follows the price of the instrument of every
//!   position, where the exchange streams it
//!   ([`Prices::follow`](crate::data::Prices::follow)): a
//!   position shows as it stands at the price heard since its report
//!   ([`Position::priced_at`](crate::data::portfolio::Position::priced_at)),
//!   so its price, its worth and its profit move with the market between
//!   two reports, and so does what the rows come to together. Which price
//!   is the settings' ([`PriceKind`](crate::data::prices::PriceKind)):
//!   the live one while they say nothing (the middle of the best bid and
//!   offer, each time either changes), or the exchange's own mark (said at
//!   the exchange's pace, about once a second on Hyperliquid). A price is
//!   shown on the frame after it is heard: nothing here holds one back. A
//!   price whose stream stops is forgotten: the row is its report again.
//!   Prices move no row from its place, and ask the exchange for nothing.
//! - An account that is the connected wallet's own
//!   ([`Wallet`](crate::data::Wallet)), on an exchange Lattice trades on
//!   ([`Trading`](crate::data::Trading)), is more than read: each of its
//!   rows can be reduced or closed, with an order at the market's price
//!   that only ever makes the position smaller
//!   ([`OrderRequest::closing`](crate::data::trading::OrderRequest::closing)).
//!   Any other account is watched by its address alone, and nothing here
//!   can act for it. From the moment the wallet connects the session
//!   follows its account live
//!   ([`Trading::follow`](crate::data::Trading::follow)), trading enabled
//!   or not, so its rows are current and show what changed them at once,
//!   and watches its open orders: those that wait to close a position are
//!   said in its row as its take profit and its stop loss. The position
//!   being closed and the size typed for it live in the session, so another
//!   tab or page leaves them as they were.
//!
//! Settings ([`PositionsSettings`]: the accounts, and the price the
//! positions are valued at) are saved in its tab record; the session keeps a copy the host brings up to date. The pane's
//! gear opens them, and so does the accounts' strip; with no account the
//! widget asks for one itself ([`view`]).
//!
//! It knows no exchange: what it shows is the portfolio's
//! ([`crate::data::portfolio`]), what it sends is trading's
//! ([`crate::data::trading`]), and what every account widget shares is in
//! [`crate::widgets::common::accounts`].

mod panel;
mod session;
mod settings;
#[cfg(test)]
mod tests;
mod view;

use std::rc::Rc;

use nsh_core::prelude::*;

use self::{session::PositionsSession, settings::PositionsSettings};
use crate::{
    data::{DataServices, SessionScope},
    widgets::{Category, TabContext, host::Widget},
};

/// The Positions, as the registry lists it.
pub(crate) struct Positions;

impl Widget for Positions {
    const ID: &'static str = "positions";
    const TITLE: &'static str = "Positions";
    const DESCRIPTION: &'static str =
        "Monitor accounts' open positions, and reduce or close your wallet's own.";
    const CATEGORY: Category = Category::Portfolio;

    type Settings = PositionsSettings;
    type Session = PositionsSession;

    fn session(
        scope: SessionScope,
        services: &DataServices,
        settings: Signal<PositionsSettings>,
    ) -> PositionsSession {
        PositionsSession::new(
            scope,
            services.portfolio.clone(),
            services.prices.clone(),
            services.trading.clone(),
            settings,
        )
    }

    /// Its table, for the account its tab's settings name.
    fn view(session: &Rc<PositionsSession>, tab: &TabContext<PositionsSettings>) -> View {
        view::positions_view(session, tab)
    }

    fn panel(
        session: &Rc<PositionsSession>,
        tab: &TabContext<PositionsSettings>,
    ) -> Option<Overlay> {
        Some(panel::panel(tab, &session.portfolio))
    }
}
