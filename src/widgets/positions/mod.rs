//! The Positions: an account's open positions, a row each with its size,
//! worth, entry and mark prices, unrealized profit and loss, liquidation
//! price and margin; or several accounts' in one table, on one exchange or
//! on several, each row marked with whose it is.
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
//! - Of several accounts it watches each one's report. Nothing is listed
//!   until each has arrived or failed, so the table never shows with an
//!   account still on its way; with some missing, the others' are listed
//!   and the frame says which are not.
//! - While it shows, each report makes the rows again: the positions worth
//!   the most first, whichever account each is of. A hidden widget makes
//!   none.
//!
//! Settings ([`PositionsSettings`]: the accounts) are saved in its tab
//! record; the session keeps a copy the host brings up to date. The pane's
//! gear opens them, and so does the accounts' strip; with no account the
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

use self::{session::PositionsSession, settings::PositionsSettings};
use crate::{
    data::{DataServices, SessionScope},
    widgets::{Category, TabContext, common::accounts::account_panel, host::Widget},
};

/// The Positions, as the registry lists it.
pub(crate) struct Positions;

impl Widget for Positions {
    const ID: &'static str = "positions";
    const TITLE: &'static str = "Positions";
    const DESCRIPTION: &'static str = "Monitor an account's open positions.";
    const CATEGORY: Category = Category::Portfolio;

    type Settings = PositionsSettings;
    type Session = PositionsSession;

    fn session(
        scope: SessionScope,
        services: &DataServices,
        settings: Signal<PositionsSettings>,
    ) -> PositionsSession {
        PositionsSession::new(scope, services.portfolio.clone(), settings)
    }

    /// Its table, for the account its tab's settings name.
    fn view(session: &Rc<PositionsSession>, tab: &TabContext<PositionsSettings>) -> View {
        view::positions_view(session, tab)
    }

    fn panel(
        session: &Rc<PositionsSession>,
        tab: &TabContext<PositionsSettings>,
    ) -> Option<Overlay> {
        Some(account_panel(
            tab,
            &session.portfolio,
            "Positions settings",
            [],
        ))
    }
}
