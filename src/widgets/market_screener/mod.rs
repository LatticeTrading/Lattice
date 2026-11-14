//! The Market Screener: every perpetual and futures market Ferris serves,
//! one row each, with prices, funding, 24h volume and open interest (the
//! old app's first widget).
//!
//! Data (the widget's session, [`Screener`] in [`session`]):
//!
//! - It watches the capabilities and, for each statistics scope they
//!   offer (an exchange's linear perpetuals, its inverse futures, ...),
//!   the scope's shared statistics query, refetched on the exchange's
//!   cadence. Two screeners share one request per scope; a hidden screener
//!   keeps polling, so showing it again is instant.
//! - While it shows, each new snapshot (or a value going stale) builds the
//!   next [`universe::Universe`], which is filtered and sorted into the
//!   rows the table shows ([`Table`](session::Table)). A hidden screener does none of it.
//! - Its settings ([`ScreenerSettings`]: scope, products, columns, their
//!   order and widths) are saved in its tab record. The session keeps a
//!   copy the host brings up to date, and polls only the scopes they
//!   choose: the same shared queries, fewer of them.
//!
//! The view ([`table`]) binds to the session's signals and the tab's
//! settings; the settings panel ([`panel`]) opens from the pane's gear.

mod columns;
mod panel;
mod session;
mod settings;
mod table;
mod universe;

use std::rc::Rc;

use nsh_core::prelude::*;

use self::{session::Screener, settings::ScreenerSettings};
use crate::{
    data::{DataServices, SessionScope},
    widgets::{Category, TabContext, host::Widget},
};

/// The Market Screener, as the registry lists it.
pub(crate) struct MarketScreener;

impl Widget for MarketScreener {
    const ID: &'static str = "market-screener";
    const TITLE: &'static str = "Market Screener";
    const DESCRIPTION: &'static str =
        "Scan and compare derivative markets in a customizable table.";
    const CATEGORY: Category = Category::Market;

    type Settings = ScreenerSettings;
    type Session = Screener;

    fn session(
        scope: SessionScope,
        services: &DataServices,
        settings: Signal<ScreenerSettings>,
    ) -> Screener {
        Screener::new(scope, services.ferris.clone(), settings)
    }

    /// Its rows, as its tab's settings say.
    fn view(screener: &Rc<Screener>, tab: &TabContext<ScreenerSettings>) -> View {
        forget_unusable_scope(screener, tab);
        table::table_view(screener.clone(), tab.clone())
    }

    fn panel(screener: &Rc<Screener>, tab: &TabContext<ScreenerSettings>) -> Option<Overlay> {
        Some(panel::panel(screener.clone(), tab.clone()))
    }
}

/// Shows every exchange again when the saved scope is of a product no
/// longer shown, or its exchange turned out not to have it (the old app
/// did both). A scope the capabilities stopped offering is kept: the view
/// says so and offers to show every exchange.
fn forget_unusable_scope(screener: &Rc<Screener>, tab: &TabContext<ScreenerSettings>) {
    let (offered, feeds, saved, tab) =
        (screener.offered, screener.feeds, tab.settings, tab.clone());
    effect(move || {
        let settings = saved.get();
        let Some(scope) = settings.scope() else {
            return;
        };
        let product = offered.with(|offered| {
            offered
                .as_ref()
                .and_then(|offered| offered.iter().find(|choice| choice.id == scope))
                .map(|choice| choice.product)
        });
        let absent = feeds.with(|feeds| {
            feeds
                .iter()
                .any(|feed| feed.scope.partition.id == scope && feed.absent())
        });
        let hidden = product.is_some_and(|product| !settings.products().contains(&product));
        if hidden || absent {
            untracked(|| tab.save(&settings.with_scope(None)));
        }
    });
}
