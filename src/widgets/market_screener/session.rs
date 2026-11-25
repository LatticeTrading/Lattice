//! The Market Screener's session ([`Screener`]): the statistics watches
//! its settings ask for, and the table its view shows (the widget's data,
//! described in the parent module).

use std::{cell::RefCell, collections::HashSet, rc::Rc, time::Duration};

use nsh_core::{prelude::*, time::unix_ms};

use super::{
    ScreenerSettings,
    columns::{self, Sort, default_sort},
};
use crate::{
    data::{
        SessionScope,
        ferris::{
            Capabilities, Ferris, MarketSelection, MarketStatsScope, Product, StatsFieldName,
            StatsSnapshot, stats_scopes,
            universe::{self, FeedSnap, Universe},
        },
        net::{ApiError, QueryState, Watch},
    },
    exchanges,
    widgets::common::{Keyed, Shared},
};

/// Every product the screener can show (`settings::PRODUCT_CHOICES`).
/// Spot and options have their own metrics and get screeners of their own.
pub(super) const PRODUCTS: [Product; 2] = [Product::Perp, Product::Future];

/// A market a watchlist shows: its exchange and the id the catalog issued
/// it, which its statistics row carries too, and its product.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct WatchKey {
    pub exchange: String,
    pub market_id: String,
    pub product: Product,
}

impl WatchKey {
    /// `market`'s key, when it is one the screener's statistics cover: a
    /// perpetual or future with a catalog id.
    pub fn of(market: &MarketSelection) -> Option<Self> {
        let product = market.kind.product();
        let market_id = market
            .market_id
            .as_deref()
            .map(str::trim)
            .unwrap_or_default();
        (PRODUCTS.contains(&product) && !market_id.is_empty()).then(|| Self {
            exchange: market.exchange.trim().to_ascii_lowercase(),
            market_id: market_id.to_owned(),
            product,
        })
    }
}

/// A scope the settings can choose: a partition the capabilities offer.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ScopeChoice {
    /// The partition id, which the settings save.
    pub id: String,
    /// `Bybit · Linear perpetuals`.
    pub label: String,
    pub product: Product,
}

/// One scope the screener polls, and its query's state.
#[derive(Clone)]
pub(super) struct Feed {
    pub scope: Rc<MarketStatsScope>,
    state: ReadSignal<QueryState<StatsSnapshot>>,
}

impl PartialEq for Feed {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.scope, &other.scope) && self.state == other.state
    }
}

impl Feed {
    /// The exchange does not have this partition after all (tracked).
    pub fn absent(&self) -> bool {
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
type Watches = Rc<Keyed<String, (Rc<MarketStatsScope>, Watch<StatsSnapshot>)>>;

/// What the table shows: the rows that match the search, in order.
pub(super) struct Table {
    pub universe: Rc<Universe>,
    /// Indices into the universe's rows.
    pub order: Vec<usize>,
    /// How many watched markets no row shows, search aside (a watchlist's;
    /// a screener's is 0).
    pub missing: usize,
}

impl Table {
    /// The rows of `universe` that match `search` (and are `watched`, for a
    /// watchlist), sorted by `sort`.
    fn project(
        universe: Rc<Universe>,
        search: &str,
        sort: Sort,
        watched: Option<&[WatchKey]>,
    ) -> Self {
        let query = search.trim().to_lowercase();
        let wanted: Option<HashSet<(&str, &str)>> = watched.map(|watched| {
            watched
                .iter()
                .map(|key| (key.exchange.as_str(), key.market_id.as_str()))
                .collect()
        });
        // The watched markets some row shows (a market two of its exchange's
        // scopes list shows twice, and is one market found).
        let mut found = HashSet::new();
        let mut order: Vec<usize> = (0..universe.rows.len())
            .filter(|&index| {
                if let Some(wanted) = &wanted {
                    let row = &universe.rows[index];
                    let key = (
                        universe.feed(row).scope.exchange.as_str(),
                        universe.market(row).market_id.as_str(),
                    );
                    if !wanted.contains(&key) {
                        return false;
                    }
                    found.insert(key);
                }
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
        let missing = wanted.map_or(0, |wanted| wanted.len().saturating_sub(found.len()));
        Self {
            universe,
            order,
            missing,
        }
    }

    /// The universe row shown at `position`, if there is one.
    pub fn row(&self, position: usize) -> Option<&universe::Row> {
        self.universe.rows.get(*self.order.get(position)?)
    }

    /// The market shown at `position`, as a watchlist keeps it.
    pub fn key(&self, position: usize) -> Option<WatchKey> {
        let row = self.row(position)?;
        let feed = self.universe.feed(row);
        Some(WatchKey {
            exchange: feed.scope.exchange.clone(),
            market_id: self.universe.market(row).market_id.clone(),
            product: feed.scope.partition.product,
        })
    }
}

/// A screener's session state: its data leases, its settings, search and
/// sort, and the table its view shows.
pub(crate) struct Screener {
    /// The capabilities: which scopes there are.
    discovery: Watch<Capabilities>,
    /// One shared statistics watch per scope, by query key.
    watches: Watches,
    /// The scopes polled, in order, and their queries' states.
    pub(super) feeds: Signal<Vec<Feed>>,
    /// Every scope the capabilities offer, of either product, whether
    /// polled or not; `None` until they are known.
    pub(super) offered: Signal<Option<Rc<Vec<ScopeChoice>>>>,
    /// The session's copy of the tab's settings, which the host keeps
    /// equal to the tab's.
    pub(super) settings: Signal<ScreenerSettings>,
    pub(super) search: Signal<String>,
    /// `None` until the user picks a column.
    pub(super) sort: Signal<Option<Sort>>,
    /// What the view shows, updated while the widget is visible.
    pub(super) table: Signal<Shared<Table>>,
    /// A watchlist's markets, which alone it polls and shows; `None` for a
    /// screener, which shows every market of its scope and products.
    pub(super) watched: Option<Memo<Vec<WatchKey>>>,
}

impl Screener {
    pub(super) fn new(
        scope: SessionScope,
        ferris: Ferris,
        settings: Signal<ScreenerSettings>,
        watched: Option<Memo<Vec<WatchKey>>>,
    ) -> Self {
        let discovery = ferris.watch_capabilities();
        let watches: Watches = Rc::default();
        let feeds = signal(Vec::<Feed>::new());
        let offered = signal(None::<Rc<Vec<ScopeChoice>>>);
        let search = signal(String::new());
        let sort = signal(None::<Sort>);
        let table = signal(Shared::new(Table {
            universe: Rc::new(Universe::empty()),
            order: Vec::new(),
            missing: 0,
        }));

        follow_scopes(
            scope,
            ferris,
            discovery.state(),
            demand(settings, watched),
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
                let projected = search.with(|search| match watched {
                    Some(watched) => watched
                        .with(|watched| Table::project(universe.0, search, sort, Some(watched))),
                    None => Table::project(universe.0, search, sort, None),
                });
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
            watched,
        }
    }

    /// The sort in effect: the user's, else the default for the products
    /// shown (tracked).
    pub(super) fn current_sort(&self) -> Sort {
        self.sort
            .get()
            .unwrap_or_else(|| self.settings.with(|s| default_sort(&s.products())))
    }

    /// The scopes the settings can choose for the products they show,
    /// leaving out any an exchange turned out not to have (tracked).
    pub(super) fn choices(&self) -> Vec<ScopeChoice> {
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
    pub(super) fn scope_unknown(&self) -> bool {
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
    pub(super) fn discovery(&self) -> ReadSignal<QueryState<Capabilities>> {
        self.discovery.state()
    }

    /// Asks for the capabilities again (after discovery failed).
    pub(super) fn retry_discovery(&self) {
        self.discovery.refresh();
        self.watches.with(|held| {
            for (_, (_, watch)) in held {
                if watch.state().with_untracked(|state| state.error.is_some()) {
                    watch.refresh();
                }
            }
        });
    }
}

/// What the settings ask to poll: the products shown, the one scope shown
/// (`None`: every exchange), and the exchanges polled (`None`: every one;
/// a watchlist's are those of its markets).
type Demand = (Vec<Product>, Option<String>, Option<Vec<String>>);

/// What `settings` ask to poll, or for a watchlist what its `watched`
/// markets do: every scope of their exchanges and products.
fn demand(
    settings: Signal<ScreenerSettings>,
    watched: Option<Memo<Vec<WatchKey>>>,
) -> Memo<Demand> {
    computed(move || match watched {
        Some(watched) => watched.with(|watched| {
            let products = PRODUCTS
                .into_iter()
                .filter(|product| watched.iter().any(|key| key.product == *product))
                .collect();
            let mut exchanges: Vec<String> =
                watched.iter().map(|key| key.exchange.clone()).collect();
            exchanges.sort();
            exchanges.dedup();
            (products, None, Some(exchanges))
        }),
        None => settings.with(|settings| {
            (
                settings.products(),
                settings.scope().map(str::to_owned),
                None,
            )
        }),
    })
}

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
    demand: Memo<Demand>,
    watches: Watches,
    feeds: Signal<Vec<Feed>>,
    offered: Signal<Option<Rc<Vec<ScopeChoice>>>>,
) {
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
        let (products, chosen, exchanges) = wanted;
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
            let shown = scopes.into_iter().filter(|scope| {
                products.contains(&scope.partition.product)
                    && chosen.as_ref().is_none_or(|id| *id == scope.partition.id)
                    && exchanges
                        .as_ref()
                        .is_none_or(|exchanges| exchanges.contains(&scope.exchange))
            });
            // A kept scope keeps its watch unless its cadence changed (the
            // new watch is made before the old one goes, so the value stays
            // cached between them); scopes no longer offered or shown are
            // let go.
            watches.follow(
                shown.map(|scope| (scope.key.clone(), Rc::new(scope))),
                |(held, _), scope| {
                    let same = held.capability.poll_interval == scope.capability.poll_interval;
                    if same {
                        *held = scope.clone();
                    }
                    same
                },
                |scope| {
                    let watch =
                        session.run(|| ferris.queries().watch(ferris.market_stats_query(scope)));
                    (scope.clone(), watch)
                },
            );
            feeds.set(watches.with(|held| {
                held.iter()
                    .map(|(_, (scope, watch))| Feed {
                        scope: scope.clone(),
                        state: watch.state(),
                    })
                    .collect()
            }));
        });
    });
}
