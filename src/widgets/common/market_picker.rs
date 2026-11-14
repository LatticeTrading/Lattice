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

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use nsh_core::{keymap::When, prelude::*};

use super::{LogoStyle, Shared, exchange_logo, market_tags};
use crate::{
    data::{
        DataServices,
        ferris::{
            Availability, Capabilities, Catalog, CatalogMarket, CatalogScope, MarketSelection,
            Requirement, SymbolQuery, catalog_scopes, merge_catalogs,
        },
        net::{ApiError, QueryState, Watch},
    },
    exchanges, theme,
    ui::{self, RowScroll},
};

/// The old modal's width and result list height.
const PANEL_WIDTH: Points = px(450.0);
const LIST_HEIGHT: f32 = 400.0;
/// A result's height.
const ROW_HEIGHT: f32 = 36.0;
/// How the exchange tabs draw logos.
const LOGO: LogoStyle = LogoStyle {
    size: 14.0,
    color: theme::TEXT,
    initials_size: 7.0,
    boxed: true,
};
/// The current selection's mark at a row's left edge.
const CURRENT_EDGE: f32 = 3.0;

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
            .child(results(
                list, scroll, shown, listing, highlight, names, current, pick,
            ));
        ui::modal_panel(PANEL_WIDTH, title, actions, on_close, body)
    }
}

/// One catalog scope's latest result: what the listing is made of (a fetch
/// starting changes nothing).
#[derive(Clone)]
struct Got {
    exchange: String,
    data: Option<Rc<Catalog>>,
    error: Option<ApiError>,
}

impl PartialEq for Got {
    fn eq(&self, other: &Self) -> bool {
        let same_data = match (&self.data, &other.data) {
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        same_data && self.exchange == other.exchange && self.error == other.error
    }
}

/// The picker's demand: the capabilities and one catalog watch per scope
/// they call for, made under the picker's owner and let go with it.
#[derive(Clone)]
struct Sources {
    capabilities: ReadSignal<QueryState<Capabilities>>,
    capabilities_watch: Rc<Watch<Capabilities>>,
    catalogs: Rc<RefCell<HashMap<String, Watch<Catalog>>>>,
    /// Each scope's latest result.
    results: Memo<Vec<Got>>,
}

impl Sources {
    fn follow(
        ferris: &crate::data::ferris::Ferris,
        exchange: Option<String>,
        requirement: Requirement,
    ) -> Self {
        let owner = Owner::current();
        let capabilities_watch = Rc::new(ferris.watch_capabilities());
        let capabilities = capabilities_watch.state();
        let catalogs: Rc<RefCell<HashMap<String, Watch<Catalog>>>> = Rc::default();
        let feeds = signal(Vec::<(CatalogScope, ReadSignal<QueryState<Catalog>>)>::new());
        {
            let (ferris, catalogs, exchange) = (ferris.clone(), catalogs.clone(), exchange.clone());
            let followed = RefCell::new(None::<Rc<Capabilities>>);
            effect(move || {
                let Some(document) = capabilities.with(|state| state.data.clone()) else {
                    return;
                };
                // Refetched every few minutes: the same document changes
                // nothing.
                if followed
                    .borrow()
                    .as_ref()
                    .is_some_and(|followed| Rc::ptr_eq(followed, &document))
                {
                    return;
                }
                *followed.borrow_mut() = Some(document.clone());
                untracked(|| {
                    let scopes = catalog_scopes(&document, exchange.as_deref(), |id, product| {
                        requirement.availability(&document, id, product)
                            != Availability::Unsupported
                    });
                    let mut previous = std::mem::take(&mut *catalogs.borrow_mut());
                    let mut next = HashMap::new();
                    let mut listed = Vec::new();
                    for scope in scopes {
                        let watch = previous.remove(&scope.key).unwrap_or_else(|| {
                            owner.run(|| ferris.queries().watch(ferris.catalog_query(&scope)))
                        });
                        listed.push((scope.clone(), watch.state()));
                        next.insert(scope.key, watch);
                    }
                    *catalogs.borrow_mut() = next;
                    // Scopes no longer called for: their demand ends here.
                    drop(previous);
                    feeds.set(listed);
                });
            });
        }
        let results = computed(move || {
            feeds.with(|feeds| {
                feeds
                    .iter()
                    .map(|(scope, state)| {
                        state.with(|state| Got {
                            exchange: scope.exchange.clone(),
                            data: state.data.clone(),
                            error: state.error.clone(),
                        })
                    })
                    .collect()
            })
        });
        Self {
            capabilities,
            capabilities_watch,
            catalogs,
            results,
        }
    }

    /// Asks again for what failed.
    fn retry(&self) {
        if self
            .capabilities
            .with_untracked(|state| state.error.is_some())
        {
            self.capabilities_watch.refresh();
        }
        for watch in self.catalogs.borrow().values() {
            if watch.state().with_untracked(|state| state.error.is_some()) {
                watch.refresh();
            }
        }
    }
}

/// What the picker can offer, from what has arrived.
#[derive(Default)]
struct Listing {
    /// Each exchange with markets on offer, in the old app's order, and
    /// its markets in list order.
    exchanges: Vec<(String, Vec<Rc<CatalogMarket>>)>,
    /// Every exchange's markets, exchange after exchange.
    all: Vec<Rc<CatalogMarket>>,
    /// Something has not answered yet.
    loading: bool,
    /// What could not load and why: exchange names, or the capabilities.
    failed: Vec<(String, ApiError)>,
}

impl Listing {
    fn new(
        capabilities: &QueryState<Capabilities>,
        got: &[Got],
        requirement: Requirement,
        only: Option<&str>,
    ) -> Self {
        let mut listing = Self::default();
        let Some(document) = capabilities.data.as_deref() else {
            listing.loading = capabilities.error.is_none();
            if let Some(error) = &capabilities.error {
                listing
                    .failed
                    .push(("the exchange list".to_owned(), error.clone()));
            }
            return listing;
        };
        let mut ids: Vec<&str> = got.iter().map(|got| got.exchange.as_str()).collect();
        ids.sort_by_key(|id| exchanges::rank(id));
        ids.dedup();
        for id in ids {
            if only.is_some_and(|only| only != id) {
                continue;
            }
            let scopes: Vec<&Got> = got.iter().filter(|got| got.exchange == id).collect();
            listing.loading |= scopes
                .iter()
                .any(|got| got.data.is_none() && got.error.is_none());
            if let Some(error) = scopes
                .iter()
                .find(|got| got.data.is_none())
                .and_then(|got| got.error.clone())
            {
                listing
                    .failed
                    .push((exchanges::display_name(id).to_owned(), error));
            }
            let markets: Vec<Rc<CatalogMarket>> = merge_catalogs(
                scopes
                    .iter()
                    .filter_map(|got| got.data.as_ref())
                    .map(|catalog| &catalog.markets),
            )
            .into_iter()
            .filter(|market| {
                requirement.availability(document, id, market.selection.kind.product())
                    != Availability::Unsupported
            })
            .collect();
            if !markets.is_empty() {
                listing.all.extend(markets.iter().cloned());
                listing.exchanges.push((id.to_owned(), markets));
            }
        }
        listing
    }

    /// The results for `tab` (`None`: every exchange) and `query`: every
    /// market in list order with nothing typed, else those matching, best
    /// first, ties in list order.
    fn shown(&self, tab: Option<&str>, query: &SymbolQuery) -> Vec<Rc<CatalogMarket>> {
        let markets = match tab {
            None => &self.all,
            Some(tab) => match self.exchanges.iter().find(|(id, _)| id == tab) {
                Some((_, markets)) => markets,
                None => &self.all,
            },
        };
        if query.is_empty() {
            return markets.clone();
        }
        let mut scored: Vec<(u8, &Rc<CatalogMarket>)> = markets
            .iter()
            .map(|market| (query.score(market), market))
            .filter(|(score, _)| *score > 0)
            .collect();
        // Stable: equal scores keep list order.
        scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        scored
            .into_iter()
            .map(|(_, market)| market.clone())
            .collect()
    }
}

/// How many tabs a row of the strip holds.
const TABS_PER_ROW: usize = 4;
/// The strip's expand button's width.
const TOGGLE_WIDTH: f32 = 34.0;

/// "All", then a tab per exchange with markets on offer, in a grid of rows
/// of [`TABS_PER_ROW`] equal tabs; a last row with fewer shares the width
/// evenly among them. Collapsed (every opening) it shows one row, the one
/// holding the selected tab, so the selection is always in view; the
/// button at its right expands it to every row and collapses it again.
fn tabs(listing: Memo<Shared<Listing>>, tab: Signal<Option<String>>) -> View {
    let ids = computed(move || {
        listing.with(|listing| {
            std::iter::once(None)
                .chain(listing.0.exchanges.iter().map(|(id, _)| Some(id.clone())))
                .collect::<Vec<_>>()
        })
    });
    let expanded = signal(false);
    let rows = computed(move || ids.with(|ids| ids.len().div_ceil(TABS_PER_ROW)));
    // The row the selected tab is in (a tab no longer listed shows "All",
    // which is in the first).
    let selected_row = computed(move || {
        let selected = tab.get();
        ids.with(|ids| {
            ids.iter()
                .position(|id| *id == selected)
                .map_or(0, |at| at / TABS_PER_ROW)
        })
    });
    let grid = dynamic(move || {
        let ids = ids.get();
        let shown: Vec<usize> = if expanded.get() {
            (0..rows.get()).collect()
        } else {
            vec![selected_row.get()]
        };
        shown
            .into_iter()
            .enumerate()
            .map(|(at, row)| {
                let cells = ids
                    .iter()
                    .skip(row * TABS_PER_ROW)
                    .take(TABS_PER_ROW)
                    .cloned()
                    .collect::<Vec<_>>();
                let last = cells.len().saturating_sub(1);
                let line = view().flex_row().w_full();
                let line = if at > 0 {
                    line.border_t_1().border_color(theme::BORDER)
                } else {
                    line
                };
                line.children(
                    cells
                        .into_iter()
                        .enumerate()
                        .map(|(cell, id)| tab_button(id, tab, cell == last)),
                )
            })
            .collect::<Vec<_>>()
    });
    let toggle = view()
        .w(px(TOGGLE_WIDTH))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .border_l_1()
        .border_color(theme::BORDER)
        .text_size(px(11.0))
        .text_color(theme::TEXT_MUTED)
        .role(AccessibilityRole::Button)
        .label(move || {
            if expanded.get() {
                "Show fewer exchanges"
            } else {
                "Show every exchange"
            }
        })
        .accessibility_state(move || AccessibilityState {
            expanded: Some(expanded.get()),
            ..Default::default()
        })
        .hover(|s| s.bg(theme::PANEL).text_color(theme::TEXT))
        .style_when(move || rows.get() < 2, |s| s.hidden())
        .on_press(move || expanded.update(|open| *open = !*open))
        .child(text(move || if expanded.get() { "▴" } else { "▾" }));
    view()
        .flex_none()
        .flex_row()
        .bg(theme::SURFACE)
        .border_b_1()
        .border_color(theme::BORDER)
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .flex_col()
                .role(AccessibilityRole::Tablist)
                .label("Exchanges")
                .child(grid),
        )
        .child(toggle)
}

/// One exchange's tab (`None`: "All"), with its logo: an equal share of
/// its row, divided from the next unless it is the row's `last`.
fn tab_button(exchange: Option<String>, tab: Signal<Option<String>>, last: bool) -> View {
    let name = exchange
        .as_deref()
        .map_or("All", exchanges::display_name)
        .to_owned();
    let active = {
        let exchange = exchange.clone();
        move || tab.with(|tab| *tab == exchange)
    };
    let (shade, mark) = (active.clone(), active.clone());
    let mut button = view()
        .relative()
        .h(px(34.0))
        .flex_1()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .px(px(8.0))
        .text_size(px(11.0))
        .text_color(move || {
            if shade() {
                theme::TEXT.get()
            } else {
                theme::TEXT_MUTED.get()
            }
        })
        .bg(move || {
            if active() {
                theme::PANEL.get()
            } else {
                theme::SURFACE.get()
            }
        })
        .role(AccessibilityRole::Tab)
        .label(name.clone())
        .accessibility_state({
            let exchange = exchange.clone();
            move || AccessibilityState {
                selected: tab.with(|tab| *tab == exchange),
                ..Default::default()
            }
        })
        .hover(|style| style.bg(theme::PANEL).text_color(theme::TEXT));
    if !last {
        button = button.border_r_1().border_color(theme::BORDER);
    }
    if let Some(id) = &exchange {
        button = button.child(exchange_logo(id, LOGO));
    }
    button
        .on_press(move || tab.set(exchange.clone()))
        .child(text(name).min_w(px(0.0)).truncate())
        .child(
            view()
                .absolute()
                .left(px(0.0))
                .right(px(0.0))
                .bottom(px(0.0))
                .h(px(2.0))
                .bg(theme::ACCENT)
                .style_when(move || !mark(), |style| style.hidden()),
        )
}

/// What could not load, with Retry; hidden while everything did.
fn failures(listing: Memo<Shared<Listing>>, sources: Sources) -> View {
    let message = computed(move || {
        listing.with(|listing| {
            let failed = &listing.0.failed;
            let (_, first) = failed.first()?;
            let names: Vec<&str> = failed.iter().map(|(name, _)| name.as_str()).collect();
            Some(format!(
                "Couldn't load {}: {}",
                names.join(", "),
                first.message
            ))
        })
    });
    // The old error message: centered red text, here with Retry under it.
    view()
        .flex_none()
        .flex_col()
        .items_center()
        .gap(px(10.0))
        .px(px(20.0))
        .py(px(40.0))
        .role(AccessibilityRole::Alert)
        .style_when(move || message.with(Option::is_none), |s| s.hidden())
        .child(
            text(move || message.get().unwrap_or_default())
                .text_center()
                .text_size(px(12.0))
                .text_color(theme::DANGER),
        )
        .child(
            view()
                .min_h(px(24.0))
                .flex()
                .items_center()
                .px(px(12.0))
                .border_1()
                .border_color(theme::BORDER)
                .text_size(px(11.0))
                .text_color(theme::TEXT_MUTED)
                .role(AccessibilityRole::Button)
                .label("Retry loading markets")
                .hover(|s| s.border_color(theme::ACCENT).text_color(theme::TEXT))
                .on_press(move || sources.retry())
                .child(text("Retry")),
        )
}

/// The results, [`LIST_HEIGHT`] tall, scrolled by the app, with their
/// scrollbar; a message instead while there are none.
#[allow(clippy::too_many_arguments)]
fn results(
    list: ElementRef,
    scroll: RowScroll,
    shown: Memo<Shared<Vec<Rc<CatalogMarket>>>>,
    listing: Memo<Shared<Listing>>,
    highlight: Signal<usize>,
    names: Memo<bool>,
    current: Option<Rc<MarketSelection>>,
    pick: Rc<dyn Fn(usize)>,
) -> View {
    let empty = move || {
        listing.with(|listing| {
            if listing.0.loading && listing.0.all.is_empty() {
                "Loading symbols..."
            } else {
                "No symbols found"
            }
        })
    };
    let rows = virtual_list(
        move || shown.with(|s| s.0.len()),
        move |at: Position| result_row(at, shown, highlight, names, current.clone(), pick.clone()),
    )
    .estimated_item_size(px(ROW_HEIGHT))
    .element_ref(&list)
    .scrollable(false)
    .scrollbar(false)
    .flex_1()
    .min_h(px(0.0))
    .on_layout(move |bounds: &Bounds<Points>| scroll.measure(bounds.size.height.0))
    .on_wheel(move |event: &WheelEvent| scroll.wheel(event))
    .role(AccessibilityRole::Listbox)
    .label("Markets")
    // The old message: centered, well clear of the edges.
    .empty(
        view()
            .px(px(20.0))
            .py(px(40.0))
            .text_center()
            .text_size(px(12.0))
            .text_color(theme::TEXT_MUTED)
            .role(AccessibilityRole::Text)
            .child(text(move || empty().to_owned())),
    );
    view()
        .h(px(LIST_HEIGHT))
        .flex_none()
        .relative()
        .flex_col()
        .child(rows)
        .child(
            scroll
                .scrollbar("Scroll markets")
                .absolute()
                .top(px(0.0))
                .bottom(px(0.0))
                .right(px(0.0)),
        )
}

/// One result, as the old `.symbol-search-result`: its pair, its
/// exchange's name under "All", its tags in boxes; shaded while it is the
/// keyboard's row (the pointer moves that row too), and shaded with a bar
/// at its left edge while it is the current selection. The last row has no
/// divider under it.
fn result_row(
    at: Position,
    shown: Memo<Shared<Vec<Rc<CatalogMarket>>>>,
    highlight: Signal<usize>,
    names: Memo<bool>,
    current: Option<Rc<MarketSelection>>,
    pick: Rc<dyn Fn(usize)>,
) -> View {
    let market = computed(move || shown.with(|s| s.0.get(at.index()).cloned().map(Shared)));
    let field = move |read: fn(&CatalogMarket) -> String| {
        move || market.with(|m| m.as_ref().map(|m| read(&m.0)).unwrap_or_default())
    };
    let is_current = computed(move || {
        market.with(|m| {
            m.as_ref()
                .zip(current.as_ref())
                .is_some_and(|(m, current)| current.same_market(&m.0.selection))
        })
    });
    let exchange = computed(move || {
        market.with(|m| {
            m.as_ref()
                .map(|m| m.0.selection.exchange.clone())
                .unwrap_or_default()
        })
    });
    let highlighted = move || highlight.get() == at.index();
    let label = move || {
        market.with(|m| {
            m.as_ref()
                .map(|m| {
                    let s = &m.0.selection;
                    format!(
                        "{} {} {}",
                        exchanges::display_name(&s.exchange),
                        s.symbol,
                        m.0.tags
                    )
                })
                .unwrap_or_default()
        })
    };
    let last = move || at.index() + 1 == shown.with(|s| s.0.len());
    let kind = move || {
        market.with(|m| {
            m.as_ref()
                .map(|m| m.0.selection.kind.label().to_owned())
                .unwrap_or_default()
        })
    };
    let detail = move || market.with(|m| m.as_ref().and_then(|m| m.0.detail.clone()));
    view()
        .relative()
        .h(px(ROW_HEIGHT))
        .w_full()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .px(px(12.0))
        .border_b_1()
        .border_color(move || {
            if last() {
                theme::BORDER.opacity(0.0).get()
            } else {
                theme::BORDER.get()
            }
        })
        .bg(move || {
            if highlighted() || is_current.get() {
                theme::SURFACE.get()
            } else {
                theme::PANEL.get()
            }
        })
        .role(AccessibilityRole::Option)
        .label(label)
        .accessibility_state(move || AccessibilityState {
            selected: is_current.get(),
            ..Default::default()
        })
        .on_mouse_enter(move |_: &MouseEvent| highlight.set(at.index()))
        .on_press(move || pick(at.index()))
        .child(
            view()
                .absolute()
                .left(px(0.0))
                .top(px(0.0))
                .bottom(px(0.0))
                .w(px(CURRENT_EDGE))
                .bg(theme::ACCENT)
                .style_when(move || !is_current.get(), |s| s.hidden()),
        )
        .child(
            text(field(|m| m.selection.symbol.clone()))
                .flex_none()
                .min_w(px(80.0))
                .text_size(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::TEXT),
        )
        // The rest of the row's width, named only under "All".
        .child(
            text(move || {
                if names.get() {
                    exchanges::display_name(&exchange.get()).to_owned()
                } else {
                    String::new()
                }
            })
            .flex_1()
            .min_w(px(0.0))
            .truncate()
            .text_size(px(11.0))
            .text_color(theme::TEXT_MUTED),
        )
        .child(market_tags(kind, detail))
}
