//! The Account Performance: what an account was worth, or what it made,
//! over a day, a week, a month or all time, as a line over time; or what
//! several accounts were and made together, on one exchange or on several.
//! It is
//! drawn with the GPU plot the charts share
//! ([`crate::widgets::common::plot`]: the scene, the shader, the viewport
//! and the view with its axes, crosshair and pointer handling).
//!
//! What it plots: the account's history as its exchange samples it
//! ([`Performance`](crate::data::portfolio::Performance)), one window of it
//! at a time ([`settings`]: the period, the whole account or its
//! perpetuals alone, and its value or its profit and loss), as a line green
//! above its baseline and red below it (no profit, or the value as the
//! window began), with the area under it filled or not. Over the plot
//! are its account and period, and what the window made and traded.
//!
//! Data (the widget's session, [`AccountPerformanceSession`] in
//! [`session`]):
//!
//! - It watches its account's history
//!   ([`Portfolio::watch_performance`](crate::data::Portfolio::watch_performance)),
//!   which the account's exchange is asked for directly and every widget
//!   watching the account shares. One report holds every window, so
//!   another period, part or measure asks for nothing. The watch follows
//!   the settings: another account is watched before the last one is let
//!   go, and with none there is no watch. A hidden widget keeps its watch,
//!   so showing it again is instant.
//! - Of several accounts it watches each one's history and plots them as
//!   one ([`Performance::combined`](crate::data::portfolio::Performance::combined)):
//!   added up at every time any of them was sampled, whichever exchange
//!   each is on. Each is plotted as soon as it arrives, none waiting for
//!   another: while some are still on their way the frame says how many
//!   show, and with some missing it says which are not.
//! - While it shows, a new report or another window marks its plot, which
//!   makes the scene its view draws at most once a frame, and at once when
//!   the viewport moves or the plot is resized. A hidden widget does none
//!   of it.
//! - The viewport lives in the session, so the plot is where it was left
//!   when its page shows again. It starts over, on the whole window, when
//!   another series is plotted.
//!
//! Settings ([`AccountPerformanceSettings`]) are saved in its tab record;
//! the session keeps a copy the host brings up to date. The pane's gear
//! opens them ([`panel`]); the account's badge over the plot opens the
//! rows that list its accounts, add to them and take from them, and the
//! period's the menu of periods;
//! with no account the widget asks for one itself ([`view`]).
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

use self::{session::AccountPerformanceSession, settings::AccountPerformanceSettings};
use crate::{
    data::{DataServices, SessionScope},
    widgets::{host::Widget, Category, TabContext},
};

/// The Account Performance, as the registry lists it.
pub(crate) struct AccountPerformance;

impl Widget for AccountPerformance {
    const ID: &'static str = "account-performance";
    const TITLE: &'static str = "Account Performance";
    const DESCRIPTION: &'static str = "Chart an account's value and profit over time.";
    const CATEGORY: Category = Category::Portfolio;

    type Settings = AccountPerformanceSettings;
    type Session = AccountPerformanceSession;

    fn session(
        scope: SessionScope,
        services: &DataServices,
        settings: Signal<AccountPerformanceSettings>,
    ) -> AccountPerformanceSession {
        AccountPerformanceSession::new(scope, services.portfolio.clone(), settings)
    }

    /// Its controls and its plot, for the account and the window its tab's
    /// settings name.
    fn view(
        session: &Rc<AccountPerformanceSession>,
        tab: &TabContext<AccountPerformanceSettings>,
    ) -> View {
        view::performance_view(session, tab)
    }

    fn panel(
        session: &Rc<AccountPerformanceSession>,
        tab: &TabContext<AccountPerformanceSettings>,
    ) -> Option<Overlay> {
        Some(panel::panel(tab, &session.portfolio))
    }
}
