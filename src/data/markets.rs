//! The app's markets: every market of every exchange Ferris lists, kept
//! once for whatever offers markets (the Everything Bar's search, the
//! market picker), and how much each trades, for whatever ranks them (the
//! old app's `MarketUniverseService` and `MarketSymbolService`).
//!
//! - The [`Directory`]: the capabilities say which exchanges list markets
//!   and which catalog scopes to ask for ([`catalog_scopes`]); each scope is
//!   the shared catalog query ([`Ferris::catalog_query`]), refreshed every
//!   minute while the directory is leased. Exchanges appear as their
//!   catalogs arrive, in the old app's order; one that fails says so and
//!   keeps the markets it had.
//! - The [`Volumes`]: each perpetual's estimated 24h USD volume, from the
//!   statistics of every perpetual scope ([`stats_scopes`], the queries the
//!   Market Screener polls, so a screener in the workspace shares them)
//!   through a [`Universe`]. They are polled only while something ranks
//!   ([`MarketsLease::rank`]): the statistics are large. Until an
//!   exchange's have answered, it is awaited ([`Volumes::awaits`]).
//!
//! [`Markets::lease`] is a consumer's hold on the directory. The first lease
//! starts it, under the data services' owner, so what it watches never
//! belongs to whichever view asked first; while nothing leases it, it asks
//! for no catalogs (the query hub keeps their values ten minutes, so the
//! next lease starts from them). The shell's Everything Bar leases it for
//! as long as the app runs, which is what keeps the search and every market
//! picker instant.

use std::{cell::OnceCell, cmp::Ordering, collections::HashMap, rc::Rc};

use nsh_core::{prelude::*, time::unix_ms};

use super::{
    Keyed, Shared,
    ferris::{
        Capabilities, Catalog, CatalogMarket, CatalogScope, Ferris, MarketStatsScope, Product,
        StatsFieldName, StatsSnapshot, SymbolQuery, VENUES, Venue, catalog_scopes, merge_catalogs,
        stats_scopes,
        universe::{FeedSnap, Universe},
    },
    net::{ApiError, Consumer, QueryState, Watch},
};

/// Who the directory's requests are for, as the connection panel names it:
/// the directory is no widget, and outlives whichever view leased it first.
const DIRECTORY: Consumer = Consumer::Part("Market directory");

/// One exchange of the [`Directory`].
pub struct Listed {
    /// The exchange id, lower case.
    pub exchange: String,
    /// Its markets, each once, in list order; none until a catalog of it
    /// arrives.
    pub markets: Vec<Rc<CatalogMarket>>,
    /// A catalog of it has not answered yet.
    pub loading: bool,
    /// Why a catalog of it could not load, while that catalog has no
    /// markets to show.
    pub error: Option<ApiError>,
}

/// Every market on offer, from what has arrived.
#[derive(Default)]
pub struct Directory {
    /// What each exchange supports; `None` until it is known.
    pub capabilities: Option<Rc<Capabilities>>,
    /// Why the capabilities could not load, while there are none.
    pub error: Option<ApiError>,
    /// Every exchange that lists markets, in the old app's order.
    pub exchanges: Vec<Listed>,
}

impl Directory {
    fn of(capabilities: Option<Rc<Capabilities>>, error: Option<ApiError>, got: &[Got]) -> Self {
        if capabilities.is_none() {
            return Self {
                error,
                ..Self::default()
            };
        }
        let mut ids: Vec<&str> = got.iter().map(|got| got.exchange.as_str()).collect();
        ids.sort_by_key(|id| rank(id));
        ids.dedup();
        let exchanges = ids
            .into_iter()
            .map(|id| {
                let scopes: Vec<&Got> = got.iter().filter(|got| got.exchange == id).collect();
                Listed {
                    exchange: id.to_owned(),
                    loading: scopes
                        .iter()
                        .any(|got| got.data.is_none() && got.error.is_none()),
                    error: scopes
                        .iter()
                        .find(|got| got.data.is_none())
                        .and_then(|got| got.error.clone()),
                    markets: merge_catalogs(
                        scopes
                            .iter()
                            .filter_map(|got| got.data.as_ref())
                            .map(|catalog| &catalog.markets),
                    ),
                }
            })
            .collect();
        Self {
            capabilities,
            error: None,
            exchanges,
        }
    }

    /// Something has not answered yet: the capabilities, or a catalog.
    pub fn loading(&self) -> bool {
        match &self.capabilities {
            None => self.error.is_none(),
            Some(_) => self.exchanges.iter().any(|listed| listed.loading),
        }
    }

    /// No market has arrived.
    pub fn is_empty(&self) -> bool {
        self.exchanges
            .iter()
            .all(|listed| listed.markets.is_empty())
    }

    /// What a search for `typed` lists, best first (the old app's
    /// `searchMarketUniverse`, which ranked matches by volume alone; here
    /// how well a market matches comes first):
    ///
    /// - Nothing typed: each exchange's [`TOP_PER_EXCHANGE`] most traded
    ///   markets, the most traded of them all first. An exchange whose
    ///   volumes are still on their way ([`Volumes::awaits`]) is left out
    ///   until they arrive, rather than listed by name and then reshuffled.
    /// - Else every market whose pair matches ([`SymbolQuery`]: case and
    ///   punctuation ignored) or whose exchange's name contains what was
    ///   typed: the whole pair first, then pairs starting with it, then
    ///   pairs containing it, then the exchange's other markets; within
    ///   each, the most traded first.
    /// - Several words that match nothing that way are an exchange and a
    ///   pair (`hyperliquid btc`): words that start an exchange's id or
    ///   name pick the exchanges, the others together are the pair.
    ///
    /// A market `volumes` has no estimate for (spot, dated futures, or
    /// every market while none has arrived) comes after those it has one
    /// for, in list order.
    pub fn search(&self, volumes: &Volumes, typed: &str) -> Vec<Rc<CatalogMarket>> {
        let query = SymbolQuery::new(typed);
        let mut at = 0_usize;
        let mut hit = |market, tier| {
            at += 1;
            Hit {
                market,
                tier,
                volume: volumes.of_market(market),
                at,
            }
        };
        let mut hits = Vec::new();
        if query.is_empty() {
            for listed in &self.exchanges {
                if volumes.awaits(&listed.exchange) {
                    continue;
                }
                let mut top: Vec<Hit<'_>> =
                    listed.markets.iter().map(|market| hit(market, 0)).collect();
                top.sort_by(Hit::order);
                top.truncate(TOP_PER_EXCHANGE);
                hits.extend(top);
            }
        } else {
            let typed = typed.trim().to_lowercase();
            for listed in &self.exchanges {
                let named = exchange_name(&listed.exchange)
                    .to_lowercase()
                    .contains(&typed);
                for market in &listed.markets {
                    let tier = pair_tier(&query, market).max(u8::from(named));
                    if tier > 0 {
                        hits.push(hit(market, tier));
                    }
                }
            }
            if hits.is_empty() {
                let (exchanges, pair) = self.worded(&typed);
                let query = SymbolQuery::new(&pair);
                for listed in &self.exchanges {
                    if !exchanges.contains(&listed.exchange.as_str()) {
                        continue;
                    }
                    for market in &listed.markets {
                        let tier = if query.is_empty() {
                            1
                        } else {
                            pair_tier(&query, market)
                        };
                        if tier > 0 {
                            hits.push(hit(market, tier));
                        }
                    }
                }
            }
        }
        hits.sort_by(Hit::order);
        hits.into_iter().map(|hit| hit.market.clone()).collect()
    }

    /// `typed` (lower case) read as an exchange and a pair: the exchanges
    /// its words name (each word that starts an exchange's id or name, or
    /// is one of its short names; several such words must agree), and its
    /// other words run together. No exchange unless there are several
    /// words and one of them names one.
    fn worded(&self, typed: &str) -> (Vec<&str>, String) {
        let words: Vec<&str> = typed.split_whitespace().collect();
        let mut exchanges: Option<Vec<&str>> = None;
        let mut pair = String::new();
        if words.len() < 2 {
            return (Vec::new(), pair);
        }
        for word in words {
            let named: Vec<&str> = self
                .exchanges
                .iter()
                .map(|listed| listed.exchange.as_str())
                .filter(|id| names_exchange(word, id))
                .collect();
            if named.is_empty() {
                pair.push_str(word);
                continue;
            }
            exchanges = Some(match exchanges {
                None => named,
                Some(held) => held.into_iter().filter(|id| named.contains(id)).collect(),
            });
        }
        (exchanges.unwrap_or_default(), pair)
    }
}

/// How many of each exchange's markets an empty search lists (the old
/// app's `DEFAULT_RESULTS_PER_EXCHANGE`).
const TOP_PER_EXCHANGE: usize = 25;

/// One market a search lists, and where it comes among them.
struct Hit<'a> {
    market: &'a Rc<CatalogMarket>,
    /// How well it matches; higher is better.
    tier: u8,
    volume: Option<f64>,
    /// Its place in the directory, which ties keep.
    at: usize,
}

impl Hit<'_> {
    /// Better matches first, then the most traded (those with no estimate
    /// last), then list order.
    fn order(a: &Self, b: &Self) -> Ordering {
        b.tier
            .cmp(&a.tier)
            .then_with(|| match (a.volume, b.volume) {
                (Some(a), Some(b)) => b.total_cmp(&a),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            })
            .then_with(|| a.at.cmp(&b.at))
    }
}

/// How well `market`'s pair matches `query`: 4 the whole pair, 3 its
/// start, 2 anywhere in it, 0 not at all ([`SymbolQuery::score`]'s steps).
fn pair_tier(query: &SymbolQuery, market: &CatalogMarket) -> u8 {
    match query.score(market) {
        0 => 0,
        score if score >= 118 => 4,
        score if score >= 100 => 3,
        _ => 2,
    }
}

/// An exchange's name for people, or its id when Lattice does not know it.
fn exchange_name(id: &str) -> &str {
    Venue::listed(id).map_or(id, |venue| venue.name)
}

/// Whether `word` (lower case, two characters at least) names the exchange
/// `id`: it starts its id or its name, or is one of its short names.
fn names_exchange(word: &str, id: &str) -> bool {
    word.len() >= 2
        && (id.starts_with(word)
            || exchange_name(id).to_lowercase().starts_with(word)
            || Venue::listed(id).is_some_and(|venue| venue.logo_aliases.contains(&word)))
}

/// Where `exchange` comes among the exchanges: the venue table's order
/// (the old app's), then any other by id.
fn rank(exchange: &str) -> (usize, &str) {
    let at = VENUES
        .iter()
        .position(|venue| venue.id == exchange)
        .unwrap_or(VENUES.len());
    (at, exchange)
}

/// How much the perpetuals trade: each one's estimated 24h volume in USD,
/// by its catalog id. Spot markets and dated futures have no estimate.
#[derive(Default)]
pub struct Volumes {
    by_market: HashMap<String, f64>,
    /// The exchanges whose statistics have not all answered yet.
    awaited: Vec<String>,
    /// Ranking was asked for before the capabilities said whose statistics
    /// to ask for: every exchange is awaited.
    undiscovered: bool,
}

impl Volumes {
    fn of(inputs: &[StatsInput], undiscovered: bool) -> Self {
        let mut awaited: Vec<String> = inputs
            .iter()
            .filter(|input| input.snapshot.is_none() && input.error.is_none())
            .map(|input| input.scope.exchange.clone())
            .collect();
        awaited.dedup();
        let feeds = inputs
            .iter()
            .map(|input| {
                FeedSnap::new(
                    input.scope.clone(),
                    input.snapshot.clone(),
                    input.error.clone(),
                )
            })
            .collect();
        let universe = Universe::build(feeds, unix_ms());
        let by_market = universe
            .rows
            .iter()
            .filter_map(|row| {
                let volume = row.usd_volume.value?;
                Some((universe.market(row).market_id.clone(), volume))
            })
            .collect();
        Self {
            by_market,
            awaited,
            undiscovered,
        }
    }

    /// Statistics asked for have not all answered: more may be ranked soon.
    pub fn pending(&self) -> bool {
        self.undiscovered || !self.awaited.is_empty()
    }

    /// Whether `exchange`'s statistics are still on their way: its markets
    /// cannot be ranked yet, but will be.
    pub fn awaits(&self, exchange: &str) -> bool {
        self.undiscovered || self.awaited.iter().any(|awaited| awaited == exchange)
    }

    /// `market`'s estimated 24h USD volume, when it has one.
    pub fn of_market(&self, market: &CatalogMarket) -> Option<f64> {
        let id = market.selection.market_id.as_deref()?;
        self.by_market.get(id).copied()
    }

    /// No market has a volume (yet).
    pub fn is_empty(&self) -> bool {
        self.by_market.is_empty()
    }
}

/// One catalog scope's watch: its exchange and its query's state.
#[derive(Clone, PartialEq)]
struct CatalogFeed {
    exchange: String,
    state: ReadSignal<QueryState<Catalog>>,
}

/// One catalog scope's latest result: what the directory is made of (a
/// fetch starting changes nothing).
#[derive(Clone)]
struct Got {
    exchange: String,
    data: Option<Rc<Catalog>>,
    error: Option<ApiError>,
}

impl PartialEq for Got {
    fn eq(&self, other: &Self) -> bool {
        same(&self.data, &other.data)
            && self.exchange == other.exchange
            && self.error == other.error
    }
}

/// One statistics scope's watch.
#[derive(Clone)]
struct StatsFeed {
    scope: Rc<MarketStatsScope>,
    state: ReadSignal<QueryState<StatsSnapshot>>,
}

impl PartialEq for StatsFeed {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.scope, &other.scope) && self.state == other.state
    }
}

/// One statistics scope's latest result (a fetch starting changes nothing).
#[derive(Clone)]
struct StatsInput {
    scope: Rc<MarketStatsScope>,
    snapshot: Option<Rc<StatsSnapshot>>,
    error: Option<ApiError>,
}

impl PartialEq for StatsInput {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.scope, &other.scope)
            && same(&self.snapshot, &other.snapshot)
            && self.error == other.error
    }
}

/// The same fetched value, or none on both sides.
fn same<T>(a: &Option<Rc<T>>, b: &Option<Rc<T>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Rc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

type Catalogs = Rc<Keyed<String, (CatalogScope, Watch<Catalog>)>>;
type Statistics = Rc<Keyed<String, (Rc<MarketStatsScope>, Watch<StatsSnapshot>)>>;

/// The directory once something leased it: its watches and what they got,
/// all made under the data services' owner.
struct Running {
    /// How many leases hold the directory, and how many of them rank.
    held: Signal<usize>,
    ranked: Signal<usize>,
    capabilities: Watch<Capabilities>,
    catalogs: Catalogs,
    statistics: Statistics,
    directory: Memo<Shared<Directory>>,
    volumes: Memo<Shared<Volumes>>,
}

impl Running {
    /// Starts the directory under the current owner (the data services').
    fn start(ferris: &Ferris) -> Self {
        let owner = Owner::current();
        let held = signal(0_usize);
        let ranked = signal(0_usize);
        let wanted = computed(move || held.get() > 0);
        let ranking = computed(move || ranked.get() > 0);
        // Kept while the services live: one small request every few minutes.
        let capabilities = DIRECTORY.scope(|| ferris.watch_capabilities());
        let discovery = capabilities.state();
        // Refetched every few minutes: the same document changes nothing.
        let document = computed(move || discovery.with(|state| state.data.clone().map(Shared)));
        let discovery_error = computed(move || discovery.with(|state| state.error.clone()));

        let catalogs = Catalogs::default();
        let catalog_feeds = signal(Vec::<CatalogFeed>::new());
        {
            let (ferris, catalogs) = (ferris.clone(), catalogs.clone());
            effect(move || {
                let (document, wanted) = (document.get(), wanted.get());
                untracked(|| {
                    let scopes = match (&document, wanted) {
                        (Some(document), true) => catalog_scopes(&document.0, None, |_, _| true),
                        _ => Vec::new(),
                    };
                    // Scopes no longer called for are let go.
                    catalogs.follow(
                        scopes.into_iter().map(|scope| (scope.key.clone(), scope)),
                        |(held, _), scope| {
                            *held = scope.clone();
                            true
                        },
                        |scope| {
                            let watch = owner.run(|| {
                                DIRECTORY
                                    .scope(|| ferris.queries().watch(ferris.catalog_query(scope)))
                            });
                            (scope.clone(), watch)
                        },
                    );
                    catalog_feeds.set(catalogs.with(|held| {
                        held.iter()
                            .map(|(_, (scope, watch))| CatalogFeed {
                                exchange: scope.exchange.clone(),
                                state: watch.state(),
                            })
                            .collect()
                    }));
                });
            });
        }
        let got = computed(move || {
            catalog_feeds.with(|feeds| {
                feeds
                    .iter()
                    .map(|feed| {
                        feed.state.with(|state| Got {
                            exchange: feed.exchange.clone(),
                            data: state.data.clone(),
                            error: state.error.clone(),
                        })
                    })
                    .collect::<Vec<_>>()
            })
        });
        let directory = computed(move || {
            let document = document.get().map(|document| document.0);
            let error = discovery_error.get();
            got.with(|got| Shared::new(Directory::of(document, error, got)))
        });

        let statistics = Statistics::default();
        let stats_feeds = signal(Vec::<StatsFeed>::new());
        {
            let (ferris, statistics) = (ferris.clone(), statistics.clone());
            effect(move || {
                let (document, ranking) = (document.get(), ranking.get());
                untracked(|| {
                    let scopes = match (&document, ranking) {
                        (Some(document), true) => {
                            stats_scopes(&document.0, &[Product::Perp], &StatsFieldName::ALL)
                        }
                        _ => Vec::new(),
                    };
                    // A kept scope keeps its watch unless its cadence changed.
                    statistics.follow(
                        scopes
                            .into_iter()
                            .map(|scope| (scope.key.clone(), Rc::new(scope))),
                        |(held, _), scope| {
                            let same =
                                held.capability.poll_interval == scope.capability.poll_interval;
                            if same {
                                *held = scope.clone();
                            }
                            same
                        },
                        |scope| {
                            let watch = owner.run(|| {
                                DIRECTORY.scope(|| {
                                    ferris.queries().watch(ferris.market_stats_query(scope))
                                })
                            });
                            (scope.clone(), watch)
                        },
                    );
                    stats_feeds.set(statistics.with(|held| {
                        held.iter()
                            .map(|(_, (scope, watch))| StatsFeed {
                                scope: scope.clone(),
                                state: watch.state(),
                            })
                            .collect()
                    }));
                });
            });
        }
        let inputs = computed(move || {
            stats_feeds.with(|feeds| {
                feeds
                    .iter()
                    .map(|feed| {
                        feed.state.with(|state| StatsInput {
                            scope: feed.scope.clone(),
                            snapshot: state.data.clone(),
                            error: state.error.clone(),
                        })
                    })
                    .collect::<Vec<_>>()
            })
        });
        let volumes = computed(move || {
            // Ranking before the capabilities say which scopes to ask for.
            let undiscovered = ranking.get()
                && document.with(Option::is_none)
                && discovery_error.with(Option::is_none);
            inputs.with(|inputs| Shared::new(Volumes::of(inputs, undiscovered)))
        });

        Self {
            held,
            ranked,
            capabilities,
            catalogs,
            statistics,
            directory,
            volumes,
        }
    }

    /// Asks again for what failed.
    fn retry(&self) {
        if self
            .capabilities
            .state()
            .with_untracked(|state| state.error.is_some())
        {
            self.capabilities.refresh();
        }
        self.catalogs.with(|held| {
            for (_, (_, watch)) in held {
                if watch.state().with_untracked(|state| state.error.is_some()) {
                    watch.refresh();
                }
            }
        });
        self.statistics.with(|held| {
            for (_, (_, watch)) in held {
                if watch.state().with_untracked(|state| state.error.is_some()) {
                    watch.refresh();
                }
            }
        });
    }
}

/// One more holder of `count`; nothing once the services are gone.
fn hold(count: Signal<usize>) {
    untracked(|| {
        if let Some(held) = count.try_get() {
            count.try_set(held + 1);
        }
    });
}

/// One holder of `count` fewer; nothing once the services are gone.
fn release(count: Signal<usize>) {
    untracked(|| {
        if let Some(held) = count.try_get() {
            count.try_set(held.saturating_sub(1));
        }
    });
}

/// The app's markets, shared by everything that offers or ranks them.
/// Cloning is cheap: every clone is the same directory.
#[derive(Clone)]
pub struct Markets(Rc<Core>);

struct Core {
    ferris: Ferris,
    /// The data services' owner: what the directory makes lives with them.
    owner: Owner,
    running: OnceCell<Running>,
}

impl Markets {
    /// The markets `ferris` lists, made under the current owner (the data
    /// services'). Nothing is asked for until something leases them.
    pub(crate) fn new(ferris: Ferris) -> Self {
        Self(Rc::new(Core {
            ferris,
            owner: Owner::current(),
            running: OnceCell::new(),
        }))
    }

    fn running(&self) -> &Running {
        let core = &self.0;
        core.running
            .get_or_init(|| core.owner.run(|| Running::start(&core.ferris)))
    }

    /// A hold on the directory: its catalogs are asked for and refreshed
    /// while any lease lives.
    pub fn lease(&self) -> MarketsLease {
        hold(self.running().held);
        MarketsLease(self.clone())
    }

    /// Asks again for what could not load (the capabilities, catalogs,
    /// statistics); nothing while nothing ever leased the directory.
    pub fn retry(&self) {
        if let Some(running) = self.0.running.get() {
            running.retry();
        }
    }
}

/// One consumer's hold on the directory (see [`Markets::lease`]). Dropping
/// it lets the directory go once no other consumer holds it.
#[must_use = "dropping a lease releases the directory"]
pub struct MarketsLease(Markets);

impl MarketsLease {
    /// Every market on offer (a memo the data services own: read it only
    /// while the lease is held).
    pub fn directory(&self) -> Memo<Shared<Directory>> {
        self.0.running().directory
    }

    /// Also ranks: the perpetuals' statistics are polled while the returned
    /// lease lives, which holds the directory too.
    pub fn rank(&self) -> RankLease {
        hold(self.0.running().ranked);
        RankLease {
            markets: self.0.clone(),
            _directory: self.0.lease(),
        }
    }

    /// Holds the directory until the current owner (a view) is cleaned up,
    /// and gives it to read meanwhile.
    pub fn release_on_cleanup(self) -> Memo<Shared<Directory>> {
        let directory = self.directory();
        cleanup(move || drop(self));
        directory
    }
}

impl Drop for MarketsLease {
    fn drop(&mut self) {
        release(self.0.running().held);
    }
}

/// One consumer's demand for the markets' volumes (see
/// [`MarketsLease::rank`]). Dropping it ends the demand.
#[must_use = "dropping a lease stops the ranking"]
pub struct RankLease {
    markets: Markets,
    _directory: MarketsLease,
}

impl RankLease {
    /// How much each perpetual trades (a memo the data services own: read
    /// it only while the lease is held).
    pub fn volumes(&self) -> Memo<Shared<Volumes>> {
        self.markets.running().volumes
    }

    /// Ranks until the current owner (a view) is cleaned up, and gives the
    /// volumes to read meanwhile.
    pub fn release_on_cleanup(self) -> Memo<Shared<Volumes>> {
        let volumes = self.volumes();
        cleanup(move || drop(self));
        volumes
    }
}

impl Drop for RankLease {
    fn drop(&mut self) {
        release(self.markets.running().ranked);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::data::ferris::catalog;

    /// A catalog row; its id is what its volume is filed under.
    fn row(exchange: &str, base: &str, quote: &str, kind: &str) -> Value {
        json!({
            "symbol": format!("{base}/{quote}"),
            "base": base,
            "quote": quote,
            "type": kind,
            "active": true,
            "marketId": id(exchange, base, quote, kind),
            "settle": if kind == "perp" { json!(quote) } else { Value::Null },
            "info": { "exchangeSymbol": format!("{base}{quote}") },
        })
    }

    fn id(exchange: &str, base: &str, quote: &str, kind: &str) -> String {
        format!("{exchange}:{kind}:{base}{quote}")
    }

    fn got(exchange: &str, rows: &[Value]) -> Got {
        Got {
            exchange: exchange.to_owned(),
            data: Some(Rc::new(
                catalog::parse(&json!(rows), exchange).expect("the rows are a catalog"),
            )),
            error: None,
        }
    }

    fn directory(got: &[Got]) -> Directory {
        Directory::of(Some(Rc::new(Capabilities::default())), None, got)
    }

    fn volumes(known: &[(String, f64)]) -> Volumes {
        Volumes {
            by_market: known.iter().cloned().collect(),
            ..Volumes::default()
        }
    }

    /// What a search listed, as `exchange PAIR TYPE`.
    fn listed(found: &[Rc<CatalogMarket>]) -> Vec<String> {
        found
            .iter()
            .map(|market| {
                let market = &market.selection;
                format!(
                    "{} {} {}",
                    market.exchange,
                    market.symbol,
                    market.kind.label()
                )
            })
            .collect()
    }

    #[test]
    fn the_directory_lists_exchanges_in_the_venue_tables_order_with_what_failed() {
        let nothing = Directory::of(None, None, &[]);
        assert!(nothing.loading() && nothing.is_empty());
        let failed = Directory::of(None, Some(ApiError::network("offline")), &[]);
        assert!(!failed.loading());

        let waiting = Got {
            exchange: "aster".to_owned(),
            data: None,
            error: None,
        };
        let refused = Got {
            exchange: "apex".to_owned(),
            data: None,
            error: Some(ApiError::network("offline")),
        };
        let directory = directory(&[
            got("hyperliquid", &[row("hyperliquid", "BTC", "USDC", "perp")]),
            waiting,
            // Two scopes of one exchange are one exchange.
            got("bybit", &[row("bybit", "BTC", "USDT", "perp")]),
            got("bybit", &[row("bybit", "BTC", "USDT", "spot")]),
            refused,
        ]);
        let ids: Vec<&str> = directory
            .exchanges
            .iter()
            .map(|listed| listed.exchange.as_str())
            .collect();
        assert_eq!(ids, ["bybit", "hyperliquid", "aster", "apex"]);
        assert_eq!(directory.exchanges[0].markets.len(), 2);
        assert!(directory.loading() && !directory.is_empty());
        assert!(directory.exchanges[2].loading && directory.exchanges[2].error.is_none());
        assert!(!directory.exchanges[3].loading && directory.exchanges[3].error.is_some());
    }

    #[test]
    fn an_empty_search_lists_each_exchanges_most_traded_markets() {
        // Thirty perpetuals, the later the more traded, and a spot pair.
        let coins: Vec<String> = (0..30).map(|n| format!("C{n:02}")).collect();
        let mut rows: Vec<Value> = coins
            .iter()
            .map(|coin| row("bybit", coin, "USDT", "perp"))
            .collect();
        rows.push(row("bybit", "AAA", "USDT", "spot"));
        let mut known: Vec<(String, f64)> = coins
            .iter()
            .enumerate()
            .map(|(n, coin)| (id("bybit", coin, "USDT", "perp"), n as f64))
            .collect();
        known.push((id("hyperliquid", "BTC", "USDC", "perp"), 12.5));
        let directory = directory(&[
            got("bybit", &rows),
            got(
                "hyperliquid",
                &[
                    row("hyperliquid", "BTC", "USDC", "perp"),
                    row("hyperliquid", "BTC", "USDC", "spot"),
                ],
            ),
        ]);

        let found = listed(&directory.search(&volumes(&known), ""));
        // Bybit's top 25 and both of Hyperliquid's, the most traded first,
        // the one nothing ranks last.
        assert_eq!(found.len(), TOP_PER_EXCHANGE + 2);
        assert_eq!(found[0], "bybit C29/USDT PERP");
        assert_eq!(found[17], "hyperliquid BTC/USDC PERP");
        assert_eq!(found[25], "bybit C05/USDT PERP");
        assert_eq!(found[26], "hyperliquid BTC/USDC SPOT");
        assert!(!found.contains(&"bybit AAA/USDT SPOT".to_owned()));

        // With no volume known, each exchange's first markets in list order.
        let unranked = listed(&directory.search(&Volumes::default(), ""));
        assert_eq!(unranked.len(), TOP_PER_EXCHANGE + 2);
        assert_eq!(unranked[0], "bybit AAA/USDT SPOT");
        assert_eq!(unranked[1], "bybit C00/USDT PERP");

        // An exchange whose volumes are on their way waits for them; a
        // typed search does not.
        let awaiting = Volumes {
            awaited: vec!["bybit".to_owned()],
            ..volumes(&known)
        };
        assert!(awaiting.pending());
        assert_eq!(
            listed(&directory.search(&awaiting, "")),
            ["hyperliquid BTC/USDC PERP", "hyperliquid BTC/USDC SPOT"]
        );
        assert_eq!(directory.search(&awaiting, "c0").len(), 10);
        let undiscovered = Volumes {
            undiscovered: true,
            ..Volumes::default()
        };
        assert!(directory.search(&undiscovered, "").is_empty());
    }

    #[test]
    fn a_typed_search_ranks_the_match_before_the_volume() {
        let directory = directory(&[
            got(
                "bybit",
                &[
                    row("bybit", "BTC", "USDT", "perp"),
                    row("bybit", "BTC", "USDT", "spot"),
                    row("bybit", "BTCDOM", "USDT", "perp"),
                    row("bybit", "WBTC", "USDT", "perp"),
                    row("bybit", "ETH", "USDT", "perp"),
                ],
            ),
            got("hyperliquid", &[row("hyperliquid", "BTC", "USDC", "perp")]),
        ]);
        let volumes = volumes(&[
            (id("bybit", "BTC", "USDT", "perp"), 50.0),
            (id("bybit", "BTCDOM", "USDT", "perp"), 60.0),
            (id("bybit", "WBTC", "USDT", "perp"), 900.0),
            (id("hyperliquid", "BTC", "USDC", "perp"), 70.0),
        ]);

        // Pairs starting with it, the most traded first, the unranked spot
        // pair after them; then the pair that only contains it, however
        // much it trades.
        assert_eq!(
            listed(&directory.search(&volumes, "btc")),
            [
                "hyperliquid BTC/USDC PERP",
                "bybit BTCDOM/USDT PERP",
                "bybit BTC/USDT PERP",
                "bybit BTC/USDT SPOT",
                "bybit WBTC/USDT PERP",
            ]
        );
        // The whole pair, with or without its slash, before the pair that
        // contains it.
        assert_eq!(
            listed(&directory.search(&volumes, " BTCusdt ")),
            [
                "bybit BTC/USDT PERP",
                "bybit BTC/USDT SPOT",
                "bybit WBTC/USDT PERP",
            ]
        );
        assert!(directory.search(&volumes, "doge").is_empty());
    }

    #[test]
    fn an_exchanges_name_finds_its_markets_and_words_name_an_exchange_and_a_pair() {
        let directory = directory(&[
            got(
                "bybit",
                &[
                    row("bybit", "BTC", "USDC", "perp"),
                    row("bybit", "HYPER", "USDT", "perp"),
                ],
            ),
            got(
                "hyperliquid",
                &[
                    row("hyperliquid", "BTC", "USDC", "perp"),
                    row("hyperliquid", "ETH", "USDC", "perp"),
                ],
            ),
        ]);
        let none = Volumes::default();

        // A pair that matches comes before the exchange's other markets.
        assert_eq!(
            listed(&directory.search(&none, "hyper")),
            [
                "bybit HYPER/USDT PERP",
                "hyperliquid BTC/USDC PERP",
                "hyperliquid ETH/USDC PERP",
            ]
        );
        // Words: an exchange (its name's start, or its short name) and a
        // pair, in either order.
        for typed in ["hyperliquid btc/usdc", "btc hyper", "hl btcusdc"] {
            assert_eq!(
                listed(&directory.search(&none, typed)),
                ["hyperliquid BTC/USDC PERP"],
                "{typed}"
            );
        }
        // An exchange alone, in words, lists its markets.
        assert_eq!(directory.search(&none, "by bit").len(), 0);
        assert_eq!(directory.search(&none, "bybit hl").len(), 0);
        assert_eq!(
            listed(&directory.search(&none, "bybit  bybit")),
            ["bybit BTC/USDC PERP", "bybit HYPER/USDT PERP"]
        );
        // Words that are a pair stay a pair.
        assert_eq!(
            listed(&directory.search(&none, "btc usdc")),
            ["bybit BTC/USDC PERP", "hyperliquid BTC/USDC PERP"]
        );
    }
}
