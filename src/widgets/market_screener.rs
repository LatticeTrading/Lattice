//! The Market Screener: every perpetual and futures market Ferris serves,
//! one row each, with prices, funding, 24h volume and open interest (the
//! old app's first widget).
//!
//! Data (the widget's session, [`Screener`]):
//!
//! - It watches the capabilities and, for each statistics scope they
//!   offer (an exchange's linear perpetuals, its inverse futures, ...),
//!   the scope's shared statistics query, refetched on the exchange's
//!   cadence. Two screeners share one request per scope; a hidden screener
//!   keeps polling, so showing it again is instant.
//! - While it shows, each new snapshot (or a value going stale) builds the
//!   next [`universe::Universe`], which is filtered and sorted into the
//!   rows the table shows ([`Table`]). A hidden screener does none of it.
//! - Its settings ([`ScreenerSettings`]: scope, products, columns, their
//!   order and widths) are saved in its tab record. The session keeps a
//!   copy the view brings up to date, and polls only the scopes they
//!   choose: the same shared queries, fewer of them.
//!
//! The view ([`table`]) binds to the session's signals and the tab's
//! settings; the settings panel ([`panel`]) opens from the pane's gear.

mod columns;
mod panel;
mod settings;
mod table;
mod universe;

pub(crate) use settings::ScreenerSettings;

use std::{cell::RefCell, collections::HashMap, rc::Rc, time::Duration};

use nsh_core::{prelude::*, time::unix_ms};

use self::{
    columns::{Sort, default_sort},
    universe::{FeedSnap, Universe},
};
use crate::{
    data::{
        DataServices, SessionScope, WidgetView,
        ferris::{
            Capabilities, Ferris, MarketStatsScope, Product, StatsFieldName, StatsSnapshot,
            stats_scopes,
        },
        net::{ApiError, QueryState, Watch},
    },
    exchanges,
    layout::TabContext,
};

/// Every product the screener can show (`settings::PRODUCT_CHOICES`).
/// Spot and options have their own metrics and get screeners of their own.
const PRODUCTS: [Product; 2] = [Product::Perp, Product::Future];

/// The screener's view of `widget`: its rows, from its session, as its
/// tab's settings say, and the settings panel while it is open.
pub(crate) fn market_screener(widget: &WidgetView, tab: &TabContext<ScreenerSettings>) -> View {
    let Some(services) = DataServices::try_get() else {
        log::error!("Market Screener: no data services above the widget");
        return view().flex_1();
    };
    let saved = tab.settings;
    let screener =
        widget.state(|scope| Screener::new(scope, services.ferris.clone(), saved.get_untracked()));
    // The tab record is where the settings live; the session follows it,
    // so what it polls follows them even while no view shows it.
    let session = screener.settings;
    effect(move || {
        let next = saved.get();
        if session.with_untracked(|current| *current != next) {
            session.set(next);
        }
    });
    forget_unusable_scope(&screener, tab);
    let panel = {
        let (screener, tab) = (screener.clone(), tab.clone());
        dynamic(move || {
            tab.settings_open()
                .then(|| panel::panel(screener.clone(), tab.clone()))
        })
    };
    table::table_view(screener, tab.clone()).child(panel)
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

/// A scope the settings can choose: a partition the capabilities offer.
#[derive(Clone, Debug, PartialEq)]
struct ScopeChoice {
    /// The partition id, which the settings save.
    id: String,
    /// `Bybit · Linear perpetuals`.
    label: String,
    product: Product,
}

/// A value shared by reference, equal only to itself: what a signal of a
/// big, immutable value (a universe, a table) holds.
struct Shared<T>(pub Rc<T>);

impl<T> Shared<T> {
    fn new(value: T) -> Self {
        Self(Rc::new(value))
    }
}

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// One scope the screener polls, and its query's state.
#[derive(Clone)]
struct Feed {
    scope: Rc<MarketStatsScope>,
    state: ReadSignal<QueryState<StatsSnapshot>>,
}

impl PartialEq for Feed {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.scope, &other.scope) && self.state == other.state
    }
}

impl Feed {
    /// The exchange does not have this partition after all (tracked).
    fn absent(&self) -> bool {
        self.state.with(|state| {
            state.data.is_none()
                && state
                    .error
                    .as_ref()
                    .is_some_and(|error| error.code == "BAD_SYMBOL")
        })
    }
}

/// A feed's latest result: equal while the data and error are the same,
/// so a fetch merely starting does not build a new universe.
struct Input {
    scope: Rc<MarketStatsScope>,
    snapshot: Option<Rc<StatsSnapshot>>,
    error: Option<ApiError>,
}

impl PartialEq for Input {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.scope, &other.scope)
            && match (&self.snapshot, &other.snapshot) {
                (Some(a), Some(b)) => Rc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
            && self.error == other.error
    }
}

/// One shared statistics watch per scope, by query key, with the scope it
/// was made for.
type Watches = Rc<RefCell<HashMap<String, (Rc<MarketStatsScope>, Watch<StatsSnapshot>)>>>;

/// What the table shows: the rows that match the search, in order.
struct Table {
    pub universe: Rc<Universe>,
    /// Indices into the universe's rows.
    pub order: Vec<usize>,
}

impl Table {
    fn project(universe: Rc<Universe>, search: &str, sort: Sort) -> Self {
        let query = search.trim().to_lowercase();
        let mut order: Vec<usize> = (0..universe.rows.len())
            .filter(|&index| {
                if query.is_empty() {
                    return true;
                }
                let market = universe.market(&universe.rows[index]);
                let haystack = format!(
                    "{} {} {} {} {}",
                    market.symbol,
                    market.base,
                    market.exchange,
                    market.category.as_deref().unwrap_or(""),
                    market.settle.as_deref().unwrap_or("")
                )
                .to_lowercase();
                haystack.contains(&query)
            })
            .collect();
        columns::sort_rows(&universe, &mut order, sort);
        Self { universe, order }
    }

    /// The universe row shown at `position`, if there is one.
    pub fn row(&self, position: usize) -> Option<&universe::Row> {
        self.universe.rows.get(*self.order.get(position)?)
    }
}

/// A screener's session state: its data leases, its settings, search and
/// sort, and the table its view shows.
struct Screener {
    /// The capabilities: which scopes there are.
    discovery: Watch<Capabilities>,
    /// One shared statistics watch per scope, by query key.
    watches: Watches,
    /// The scopes polled, in order, and their queries' states.
    feeds: Signal<Vec<Feed>>,
    /// Every scope the capabilities offer, of either product, whether
    /// polled or not; `None` until they are known.
    offered: Signal<Option<Rc<Vec<ScopeChoice>>>>,
    /// The tab's settings, as the view last saw them.
    pub settings: Signal<ScreenerSettings>,
    pub search: Signal<String>,
    /// `None` until the user picks a column.
    pub sort: Signal<Option<Sort>>,
    /// What the view shows, updated while the widget is visible.
    pub table: Signal<Shared<Table>>,
}

impl Screener {
    fn new(scope: SessionScope, ferris: Ferris, saved: ScreenerSettings) -> Self {
        let discovery = ferris.watch_capabilities();
        let watches: Watches = Rc::default();
        let feeds = signal(Vec::<Feed>::new());
        let offered = signal(None::<Rc<Vec<ScopeChoice>>>);
        let settings = signal(saved);
        let search = signal(String::new());
        let sort = signal(None::<Sort>);
        let table = signal(Shared::new(Table {
            universe: Rc::new(Universe::empty()),
            order: Vec::new(),
        }));

        follow_scopes(
            scope,
            ferris,
            discovery.state(),
            settings,
            watches.clone(),
            feeds,
            offered,
        );

        scope.while_visible(move || {
            let inputs = computed(move || {
                feeds.with(|feeds| {
                    feeds
                        .iter()
                        .map(|feed| {
                            feed.state.with(|state| Input {
                                scope: feed.scope.clone(),
                                snapshot: state.data.clone(),
                                error: state.error.clone(),
                            })
                        })
                        .collect::<Vec<_>>()
                })
            });
            // Bumped when a value goes stale with no new data.
            let tick = signal(0.0_f64);
            let universe = signal(Shared::new(Universe::empty()));
            effect(move || {
                tick.get();
                let snaps = inputs.with(|inputs| {
                    inputs
                        .iter()
                        .map(|input| {
                            FeedSnap::new(
                                input.scope.clone(),
                                input.snapshot.clone(),
                                input.error.clone(),
                            )
                        })
                        .collect()
                });
                universe.set(Shared::new(Universe::build(snaps, unix_ms())));
            });
            effect(move || {
                let Some(due) = universe.with(|universe| universe.0.next_change) else {
                    return;
                };
                // A little past the deadline, so the value is stale by then.
                let delay = (due - unix_ms()).max(0.0) + 50.0;
                spawn(async move {
                    timer(Duration::from_millis(delay as u64)).await;
                    tick.try_set(unix_ms());
                });
            });
            let products = computed(move || settings.with(ScreenerSettings::products));
            effect(move || {
                let universe = universe.get();
                let sort = sort
                    .get()
                    .unwrap_or_else(|| products.with(|p| default_sort(p)));
                let projected = search.with(|search| Table::project(universe.0, search, sort));
                table.set(Shared::new(projected));
            });
        });

        Self {
            discovery,
            watches,
            feeds,
            offered,
            settings,
            search,
            sort,
            table,
        }
    }

    /// The sort in effect: the user's, else the default for the products
    /// shown (tracked).
    pub fn current_sort(&self) -> Sort {
        self.sort
            .get()
            .unwrap_or_else(|| self.settings.with(|s| default_sort(&s.products())))
    }

    /// The scopes the settings can choose for the products they show,
    /// leaving out any an exchange turned out not to have (tracked).
    pub fn choices(&self) -> Vec<ScopeChoice> {
        let products = self.settings.with(ScreenerSettings::products);
        let absent: Vec<String> = self.feeds.with(|feeds| {
            feeds
                .iter()
                .filter(|feed| feed.absent())
                .map(|feed| feed.scope.partition.id.clone())
                .collect()
        });
        self.offered.with(|offered| {
            offered
                .iter()
                .flat_map(|offered| offered.iter())
                .filter(|choice| products.contains(&choice.product) && !absent.contains(&choice.id))
                .cloned()
                .collect()
        })
    }

    /// The saved scope is one the capabilities no longer offer (tracked).
    pub fn scope_unknown(&self) -> bool {
        let scope = self.settings.with(|s| s.scope().map(str::to_owned));
        let Some(scope) = scope else {
            return false;
        };
        self.offered.with(|offered| {
            offered
                .as_ref()
                .is_some_and(|offered| !offered.iter().any(|choice| choice.id == scope))
        })
    }

    /// The capabilities' state: loading, failed, or known.
    pub fn discovery(&self) -> ReadSignal<QueryState<Capabilities>> {
        self.discovery.state()
    }

    /// Asks for the capabilities again (after discovery failed).
    pub fn retry_discovery(&self) {
        self.discovery.refresh();
        for (_, watch) in self.watches.borrow().values() {
            if watch.state().with_untracked(|state| state.error.is_some()) {
                watch.refresh();
            }
        }
    }
}

/// What the settings ask to poll: the products shown, and the one scope
/// shown (`None`: every exchange).
type Demand = (Vec<Product>, Option<String>);

/// Keeps one statistics watch per scope the capabilities offer that the
/// settings show, and `feeds` listing them in order: a new scope is
/// watched, a scope no longer offered or shown is let go (the shared query
/// keeps its value for a while, so showing it again soon is instant), and
/// a kept scope keeps its watch and its data. `offered` lists every scope
/// on offer for the settings to choose from. Runs under the session, so the
/// watches live as long as it does.
fn follow_scopes(
    session: SessionScope,
    ferris: Ferris,
    discovery: ReadSignal<QueryState<Capabilities>>,
    settings: Signal<ScreenerSettings>,
    watches: Watches,
    feeds: Signal<Vec<Feed>>,
    offered: Signal<Option<Rc<Vec<ScopeChoice>>>>,
) {
    let demand: Memo<Demand> = computed(move || {
        settings.with(|settings| (settings.products(), settings.scope().map(str::to_owned)))
    });
    let followed: RefCell<Option<(Rc<Capabilities>, Demand)>> = RefCell::new(None);
    effect(move || {
        let Some(capabilities) = discovery.with(|state| state.data.clone()) else {
            return;
        };
        let wanted = demand.get();
        // The capabilities are refetched every few minutes; the same
        // document and the same demand change nothing.
        let same_document = followed
            .borrow()
            .as_ref()
            .is_some_and(|(followed, _)| Rc::ptr_eq(followed, &capabilities));
        let same_demand = followed
            .borrow()
            .as_ref()
            .is_some_and(|(_, followed)| *followed == wanted);
        if same_document && same_demand {
            return;
        }
        *followed.borrow_mut() = Some((capabilities.clone(), wanted.clone()));
        let (products, chosen) = wanted;
        untracked(|| {
            let scopes = stats_scopes(&capabilities, &PRODUCTS, &StatsFieldName::ALL);
            if !same_document {
                let choices = scopes
                    .iter()
                    .map(|scope| ScopeChoice {
                        id: scope.partition.id.clone(),
                        label: format!(
                            "{} · {}",
                            exchanges::display_name(&scope.exchange),
                            scope.partition.label
                        ),
                        product: scope.partition.product,
                    })
                    .collect();
                offered.set(Some(Rc::new(choices)));
            }
            let mut previous = std::mem::take(&mut *watches.borrow_mut());
            let mut next = HashMap::new();
            let mut listed = Vec::new();
            let shown = scopes.into_iter().filter(|scope| {
                products.contains(&scope.partition.product)
                    && chosen.as_ref().is_none_or(|id| *id == scope.partition.id)
            });
            for scope in shown {
                let scope = Rc::new(scope);
                // A kept scope keeps its watch unless its cadence changed;
                // a new watch is made before the old one goes, so the value
                // stays cached between them.
                let watch = match previous.remove(&scope.key) {
                    Some((old, watch))
                        if old.capability.poll_interval == scope.capability.poll_interval =>
                    {
                        watch
                    }
                    _ => session.run(|| ferris.queries().watch(ferris.market_stats_query(&scope))),
                };
                listed.push(Feed {
                    scope: scope.clone(),
                    state: watch.state(),
                });
                next.insert(scope.key.clone(), (scope, watch));
            }
            *watches.borrow_mut() = next;
            // Scopes no longer offered or shown: their demand ends here.
            drop(previous);
            feeds.set(listed);
        });
    });
}
