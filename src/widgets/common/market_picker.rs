//! The market picker (the old app's `SymbolSearch`): every market of every
//! exchange Ferris lists, searched by pair and filtered by exchange; picking
//! one hands its [`MarketSelection`] to the caller and closes the picker.
//!
//! - Data: the capabilities say which exchanges list markets and which
//!   catalog scopes to ask for ([`catalog_scopes`]); each scope is the
//!   shared catalog query ([`crate::data::ferris::Ferris::catalog_query`]),
//!   watched while the picker is open and kept by the query hub for ten
//!   minutes after, so reopening it is instant. Exchanges show as their
//!   catalogs arrive; one that fails says so above the results, with Retry.
//! - Options ([`MarketPicker`]): one exchange only (the old
//!   `filterExchange`), a capability requirement (only markets whose
//!   exchange and product are not denied it: unknown metadata keeps the old
//!   behavior and offers them), the current selection (marked), and a
//!   "Clear" action.
//! - Search ignores case and punctuation ("btcusd" finds "BTC/USD") and
//!   ranks as the old app did ([`SymbolQuery`]): the whole pair, then the
//!   pair as typed, then its start, then anywhere, ties in list order.
//! - Exchange tabs: "All", then each exchange with markets on offer; rows
//!   name their exchange only under "All".
//! - The results are a `virtual_list` scrolled by the app ([`RowScroll`]),
//!   so a row never shows blank: thousands of rows under "All".
//! - Keys: the field has the focus when the picker opens; Up and Down move
//!   through the results (the list follows), Enter picks, Escape closes.
//!
//! It opens in the middle of the window ([`ui::centered_panel`]), in an
//! overlay of its own, so it can open over another floating panel (a
//! widget's settings).
//!
//! Its parts: [`sources`] (the watches and what they got), [`listing`]
//! (what can be offered, and the results for a tab and a search), [`tabs`]
//! and [`results`] (the views).

mod listing;
mod results;
mod sources;
mod tabs;

use std::rc::Rc;

use nsh_core::{keymap::When, prelude::*};

use self::{
    listing::Listing,
    results::{Results, failures},
    sources::Sources,
    tabs::tabs,
};
use super::Shared;
use crate::{
    data::{
        DataServices,
        ferris::{MarketSelection, Requirement, SymbolQuery},
    },
    theme,
    ui::{self, RowScroll},
};

/// The old modal's width.
const PANEL_WIDTH: Points = px(450.0);
/// A result's height.
const ROW_HEIGHT: f32 = 36.0;

/// A picker's options and what it does with a pick.
pub struct MarketPicker {
    exchange: Option<String>,
    requirement: Requirement,
    current: Option<MarketSelection>,
    on_pick: Rc<dyn Fn(MarketSelection)>,
    clear: Option<(String, Rc<dyn Fn()>)>,
}

impl MarketPicker {
    /// A picker of every market, calling `on_pick` with the one picked.
    pub fn new(on_pick: impl Fn(MarketSelection) + 'static) -> Self {
        Self {
            exchange: None,
            requirement: Requirement::default(),
            current: None,
            on_pick: Rc::new(on_pick),
            clear: None,
        }
    }

    /// Only `exchange`'s markets (the Liquidation Feed: Hyperliquid).
    #[allow(dead_code, reason = "for the Liquidation Feed, not ported yet")]
    pub fn exchange(mut self, exchange: &str) -> Self {
        self.exchange = Some(exchange.trim().to_ascii_lowercase());
        self
    }

    /// Only markets whose exchange and product are not denied
    /// `requirement`.
    pub fn requires(mut self, requirement: Requirement) -> Self {
        self.requirement = requirement;
        self
    }

    /// Marks `market` as the current selection.
    pub fn current(mut self, market: Option<MarketSelection>) -> Self {
        self.current = market;
        self
    }

    /// While there is a current selection, a "Clear" button (`label` for
    /// assistive tech) calls `on_clear` and closes the picker.
    #[allow(
        dead_code,
        reason = "for single-market settings (charts, books), not ported yet"
    )]
    pub fn on_clear(mut self, label: &str, on_clear: impl Fn() + 'static) -> Self {
        self.clear = Some((label.to_owned(), Rc::new(on_clear)));
        self
    }

    /// The picker, a modal in the middle of the window. A pick, Clear,
    /// Escape, its close button and a press beside it call `on_close`;
    /// leaving it out of the view is the caller's job.
    pub fn view(self, on_close: impl Fn() + Clone + 'static) -> View {
        let title = "Search for symbol";
        let Some(services) = DataServices::try_get() else {
            log::error!("Market picker: no data services above it");
            let body = view()
                .p(px(12.0))
                .text_size(px(11.0))
                .text_color(theme::TEXT_MUTED)
                .child(text("Market data is unavailable."));
            return ui::modal_panel(PANEL_WIDTH, title, None, on_close, body);
        };
        let ferris = services.ferris.clone();
        let Self {
            exchange,
            requirement,
            current,
            on_pick,
            clear,
        } = self;
        let current = current.map(Rc::new);
        let close: Rc<dyn Fn()> = Rc::new(on_close.clone());
        let every_exchange = exchange.is_none();

        let sources = Sources::follow(&ferris, exchange.clone(), requirement);
        let listing = {
            let sources = sources.clone();
            computed(move || {
                let got = sources.results.get();
                sources.capabilities.with(|state| {
                    Shared(Rc::new(Listing::new(
                        state,
                        &got,
                        requirement,
                        exchange.as_deref(),
                    )))
                })
            })
        };

        let query = signal(String::new());
        // `None` is "All".
        let tab = signal(None::<String>);
        let shown = computed(move || {
            let query = SymbolQuery::new(&query.get());
            let tab = tab.get();
            listing.with(|listing| Shared(Rc::new(listing.0.shown(tab.as_deref(), &query))))
        });
        // A tab whose exchange has nothing on offer any more shows "All".
        effect(move || {
            let gone = tab.with(|tab| {
                tab.as_ref().is_some_and(|id| {
                    listing.with(|listing| !listing.0.exchanges.iter().any(|(e, _)| e == id))
                })
            });
            if gone {
                tab.set(None);
            }
        });

        let list = element_ref();
        let scroll = RowScroll::new(list, ROW_HEIGHT, move || shown.with(|s| s.0.len()));
        // The keyboard's row: the first after every new search, tab or
        // result count (not after a refresh that lists the same).
        let highlight = signal(0_usize);
        let reset = computed(move || (query.get(), tab.get(), shown.with(|s| s.0.len())));
        effect(move || {
            reset.with(|_| ());
            highlight.set(0);
            scroll.to(0.0);
        });

        let pick: Rc<dyn Fn(usize)> = {
            let close = close.clone();
            Rc::new(move |index| {
                let market = shown.with_untracked(|s| s.0.get(index).cloned());
                if let Some(market) = market {
                    on_pick(market.selection.clone());
                    close();
                }
            })
        };
        let step = move |by: isize| {
            let count = shown.with_untracked(|s| s.0.len());
            if count == 0 {
                return;
            }
            let next = highlight
                .get_untracked()
                .saturating_add_signed(by)
                .min(count - 1);
            highlight.set(next);
            scroll.reveal(next);
        };
        let keys = KeyOptions {
            when: When::Always,
            repeat: true,
            ..KeyOptions::default()
        };
        on_key_with("down", keys, move |_| step(1));
        on_key_with("up", keys, move |_| step(-1));

        let field = {
            let pick = pick.clone();
            // The old field is the row itself: 40pt, 13pt text, lighter
            // while it has the focus.
            ui::text_field(query)
                .auto_focus(true)
                .placeholder("Search symbols...")
                .label("Search symbols")
                .flex_1()
                .h(px(40.0))
                .px(px(12.0))
                .text_size(px(13.0))
                .focus(|s| s.bg(theme::PANEL))
                .on_submit(move |_: &str| pick(highlight.get_untracked()))
        };
        let search = view()
            .flex_none()
            .flex_row()
            .border_b_1()
            .border_color(theme::BORDER)
            .child(field);
        // The old header's Clear, before the close button.
        let actions = match (&current, clear) {
            (Some(_), Some((label, on_clear))) => {
                let close = close.clone();
                Some(
                    view()
                        .h_full()
                        .flex_none()
                        .flex()
                        .items_center()
                        .px(px(12.0))
                        .border_l_1()
                        .border_color(theme::BORDER)
                        .text_color(theme::TEXT_MUTED)
                        .role(AccessibilityRole::Button)
                        .label(label)
                        .hover(|s| s.bg(theme::PANEL).text_color(theme::DANGER))
                        .on_press(move || {
                            on_clear();
                            close();
                        })
                        .child(text("Clear")),
                )
            }
            _ => None,
        };

        // Rows name their exchange only under "All" of a picker of every
        // exchange.
        let names = computed(move || every_exchange && tab.with(Option::is_none));
        let body = view()
            .test_id("market-picker")
            .flex_col()
            .text_color(theme::TEXT)
            .child(search)
            .child(tabs(listing, tab))
            .child(failures(listing, sources))
            .child(
                Results {
                    list,
                    scroll,
                    shown,
                    listing,
                    highlight,
                    names,
                    current,
                    pick,
                }
                .view(),
            );
        ui::modal_panel(PANEL_WIDTH, title, actions, on_close, body)
    }
}
