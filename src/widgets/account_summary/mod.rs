//! The Account Summary: where an account stands. What the whole of it is
//! worth and made in a day, and of what (perpetuals, spot, staked, vaults);
//! how near its cross positions are to liquidation; the margin they hold;
//! its exposure long and short; its profit and loss and funding; its fee
//! tier and rates; what it has staked and in vaults; and the assets it
//! holds, at their prices. Or where several accounts stand together, on one
//! exchange or on several.
//!
//! Data (the widget's session, [`AccountSummarySession`] in [`session`]):
//!
//! - It watches its account's reports through the
//!   [`Portfolio`](crate::data::Portfolio): its margin, balances, history,
//!   staking, vaults and fees, and its venue's token prices. The account's
//!   exchange is asked for each directly, and every widget watching the
//!   account shares the request (the Positions shows the same margin
//!   report, the Account Performance the same history; the prices are one
//!   request for every account). The watches follow the settings: another
//!   account is watched before the last one is let go. A hidden widget
//!   keeps its watches, so showing it again is instant. A report the venue
//!   does not make reads as dashes.
//! - Of several accounts it watches each one's reports and makes one
//!   overview of them (described in [`session`]): what they are worth and
//!   made added up, each one's holdings at its own exchange's prices, the
//!   margin ratio of the one nearest liquidation, and no fee schedule when
//!   several report theirs. An account counts as soon as its own reports
//!   arrive, none waiting for another's: the frame says how many show
//!   meanwhile, and with some missing which are not.
//! - While it shows, the reports make its overview (every figure, and the
//!   fractions its bars are drawn to) and its holdings again. A hidden
//!   widget makes none.
//!
//! Settings ([`AccountSummarySettings`]: the accounts, whether the balances
//! show and whether small ones are hidden) are saved in its tab record; the
//! session keeps a copy the host brings up to date. The pane's gear opens
//! them ([`panel`]), and so does the accounts' strip; with no account the
//! widget asks for one itself ([`view`]).
//!
//! It knows no exchange: what it shows is the portfolio's
//! ([`crate::data::portfolio`]), and what every account widget shares is
//! in [`crate::widgets::common::accounts`].

mod panel;
mod session;
mod settings;
#[cfg(test)]
mod tests;
mod view;

use std::rc::Rc;

use nsh_core::prelude::*;

use self::{session::AccountSummarySession, settings::AccountSummarySettings};
use crate::{
    data::{DataServices, SessionScope},
    widgets::{Category, TabContext, host::Widget},
};

/// The Account Summary, as the registry lists it.
pub(crate) struct AccountSummary;

impl Widget for AccountSummary {
    const ID: &'static str = "account-summary";
    const TITLE: &'static str = "Account Summary";
    const DESCRIPTION: &'static str =
        "Monitor an account's value, margin health, exposure, fees and balances.";
    const CATEGORY: Category = Category::Portfolio;

    type Settings = AccountSummarySettings;
    type Session = AccountSummarySession;

    fn session(
        scope: SessionScope,
        services: &DataServices,
        settings: Signal<AccountSummarySettings>,
    ) -> AccountSummarySession {
        AccountSummarySession::new(scope, services.portfolio.clone(), settings)
    }

    /// Its figures and holdings, for the account its tab's settings name.
    fn view(session: &Rc<AccountSummarySession>, tab: &TabContext<AccountSummarySettings>) -> View {
        view::summary_view(session, tab)
    }

    fn panel(
        session: &Rc<AccountSummarySession>,
        tab: &TabContext<AccountSummarySettings>,
    ) -> Option<Overlay> {
        Some(panel::panel(tab, &session.portfolio))
    }
}
