//! The Funding Rates: a watchlist of perpetuals, each with its funding
//! rate restated per hour, eight hours, day and year, how often it is
//! paid, and how old the figure is (the old app's `FundingRates`).
//!
//! Data (the widget's session, [`Funding`] in [`session`]):
//!
//! - It watches the capabilities and, for each exchange with a watched
//!   market, that exchange's perpetual statistics scopes, asking for the
//!   `funding` field alone ([`Ferris::market_stats_query`](crate::data::ferris::Ferris::market_stats_query)):
//!   one shared request per scope, however many widgets watch it. The old
//!   app polled every exchange so its picker could list their markets; the
//!   picker here reads the app's market directory, so an exchange nobody
//!   watches is not polled. A hidden widget keeps its watches, so showing
//!   it again is instant.
//! - A watched market's statistics are found by its exchange and catalog
//!   id. While it shows, the session makes the rows again whenever
//!   statistics arrive and once a second: ages count up, and a value turns
//!   stale when its exchange's freshness window passes. A hidden widget
//!   does none of it.
//!
//! Settings ([`FundingRatesSettings`]: the markets, and a switch per
//! column) are saved in its tab record; the session keeps a copy the host
//! brings up to date, so what it polls follows them while no view shows
//! it. The table edits the markets ([`view`]); the pane's gear opens the
//! switches ([`panel`]).

mod panel;
mod session;
mod settings;
#[cfg(test)]
mod tests;
mod view;

use std::rc::Rc;

use nsh_core::prelude::*;

use self::{session::Funding, settings::FundingRatesSettings};
use crate::{
    data::{DataServices, SessionScope},
    widgets::{Category, TabContext, host::Widget},
};

/// The Funding Rates, as the registry lists it.
pub(crate) struct FundingRates;

impl Widget for FundingRates {
    const ID: &'static str = "funding-rates";
    const TITLE: &'static str = "Funding Rates";
    const DESCRIPTION: &'static str = "Compare perpetual funding rates across exchanges.";
    const CATEGORY: Category = Category::Market;

    type Settings = FundingRatesSettings;
    type Session = Funding;

    fn session(
        scope: SessionScope,
        services: &DataServices,
        settings: Signal<FundingRatesSettings>,
    ) -> Funding {
        Funding::new(scope, services.ferris.clone(), settings)
    }

    /// Its table, for the markets and columns its tab's settings say.
    fn view(funding: &Rc<Funding>, tab: &TabContext<FundingRatesSettings>) -> View {
        view::funding_view(funding, tab)
    }

    fn panel(_funding: &Rc<Funding>, tab: &TabContext<FundingRatesSettings>) -> Option<Overlay> {
        Some(panel::panel(tab.clone()))
    }
}
