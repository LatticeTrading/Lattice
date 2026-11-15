//! The Funding Rates' session ([`Funding`]): the statistics watches its
//! markets need, and the rows its view shows (the widget's data, described
//! in the parent module).

use std::{cell::RefCell, rc::Rc, time::Duration};

use nsh_core::{prelude::*, time::unix_ms};

use super::settings::{FundingRatesSettings, PairKey, Period};
use crate::{
    data::{
        SessionScope,
        ferris::{
            Capabilities, Ferris, MarketSelection, MarketStatsScope, Product, StatsFieldName,
            StatsSnapshot,
            market_stats::{Decimal, FieldState, Funding as FundingValue, StatsMarket},
            stats_scopes,
            universe::FeedSnap,
        },
        net::{QueryState, Watch},
    },
    exchanges, format,
    widgets::common::{Keyed, Shared},
};

/// Funding is a perpetual's: the only product asked for.
const PRODUCTS: [Product; 1] = [Product::Perp];
/// The one field asked for, so a scope's request stays small.
const FIELDS: [StatsFieldName; 1] = [StatsFieldName::Funding];
/// How often the rows are made again while shown: ages count seconds, and
/// a value turns stale without new data.
const TICK: Duration = Duration::from_secs(1);
/// What a cell with nothing to show reads.
pub(super) const NONE: &str = "—";

/// Which way a rate reads: what colors it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Sign {
    Positive,
    Negative,
    Zero,
}

/// One period's rate as its cell shows it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Rate {
    /// `0.00125%`.
    pub text: String,
    pub sign: Sign,
}

impl Rate {
    /// An API-supplied percentage for display (the old app's
    /// `formatFundingPercentage`, and how it told the sign: from what
    /// shows, so a rate that rounds to zero has none).
    fn of(percent: &Decimal) -> Self {
        let text = format!("{}%", format::funding_rate(percent.as_str()));
        let sign = if text == "0%" {
            Sign::Zero
        } else if text.starts_with('-') {
            Sign::Negative
        } else {
            Sign::Positive
        };
        Self { text, sign }
    }
}

/// One watched market's row.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Row {
    pub key: PairKey,
    pub market: Rc<MarketSelection>,
    /// Each period's rate, in [`Period::ALL`]'s order; `None` while the
    /// funding is not available (or stale) or the API restates none.
    pub rates: [Option<Rate>; 4],
    /// How often funding is paid (`8h`).
    pub interval: String,
    /// How long ago the backend received the funding (`12s`).
    pub age: String,
    /// When it did, as ISO 8601.
    pub received: String,
}

impl Row {
    /// `period`'s rate.
    pub fn rate(&self, period: Period) -> Option<&Rate> {
        let at = Period::ALL.iter().position(|kept| *kept == period)?;
        self.rates[at].as_ref()
    }
}

/// A watched market, with what its row is known and ordered by.
#[derive(Clone, PartialEq)]
pub(super) struct Watched {
    key: PairKey,
    market: Rc<MarketSelection>,
}

/// What the widget says above its rows, besides them.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Status {
    /// The capabilities have not answered yet.
    Loading,
    /// They could not be read, and why.
    Failed(String),
    /// No exchange serves funding statistics.
    Unsupported,
    Ready,
}

/// One scope the session polls, and its query's state.
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

/// One shared statistics watch per scope, by query key, with the scope it
/// was made for.
type Watches = Rc<Keyed<String, (Rc<MarketStatsScope>, Watch<StatsSnapshot>)>>;

/// A Funding Rates widget's session state: what its view shows (its
/// watches live in the effects `new` starts).
pub(crate) struct Funding {
    /// Held so the capabilities stay watched, and refreshed, for as long as
    /// the session lives.
    _discovery: Watch<Capabilities>,
    /// What to say above the rows.
    pub(super) status: Memo<Status>,
    /// The rows, in the table's order, updated while the widget is visible.
    pub(super) rows: Signal<Shared<Vec<Row>>>,
}

impl Funding {
    /// A session following `settings`, the session's copy of the tab's.
    pub(super) fn new(
        scope: SessionScope,
        ferris: Ferris,
        settings: Signal<FundingRatesSettings>,
    ) -> Self {
        let discovery = ferris.watch_capabilities();
        let found = discovery.state();
        let feeds = signal(Vec::<Feed>::new());
        // Whether any exchange serves funding statistics; `None` until the
        // capabilities are known.
        let offered = signal(None::<bool>);
        let watched = computed(move || settings.with(|settings| ordered(settings.markets())));
        let exchanges = computed(move || {
            let mut exchanges: Vec<String> = watched.with(|watched| {
                watched
                    .iter()
                    .map(|watched| watched.market.exchange.clone())
                    .collect()
            });
            exchanges.sort();
            exchanges.dedup();
            exchanges
        });
        follow_exchanges(scope, ferris, found, exchanges, feeds, offered);

        let status = computed(move || {
            let failed = found.with(|state| match (&state.data, &state.error) {
                (None, None) => Some(Status::Loading),
                (None, Some(error)) => Some(Status::Failed(error.message.clone())),
                _ => None,
            });
            match (failed, offered.get()) {
                (Some(status), _) => status,
                (None, Some(false)) => Status::Unsupported,
                _ => Status::Ready,
            }
        });

        let rows = signal(Shared::new(Vec::new()));
        scope.while_visible(move || {
            // Bumped every second while there are rows to age.
            let tick = signal(0_u64);
            effect(move || {
                tick.get();
                let now = unix_ms();
                let snaps: Vec<FeedSnap> = feeds.with(|feeds| {
                    feeds
                        .iter()
                        .map(|feed| {
                            feed.state.with(|state| {
                                FeedSnap::new(
                                    feed.scope.clone(),
                                    state.data.clone(),
                                    state.error.clone(),
                                )
                            })
                        })
                        .collect()
                });
                let next = watched.with(|watched| rows_of(watched, &snaps, now));
                rows.set(Shared::new(next));
            });
            spawn(async move {
                let mut ticks = 0_u64;
                loop {
                    timer(TICK).await;
                    if watched.with_untracked(Vec::is_empty) {
                        continue;
                    }
                    ticks = ticks.wrapping_add(1);
                    tick.try_set(ticks);
                }
            });
        });

        Self {
            _discovery: discovery,
            status,
            rows,
        }
    }
}

/// `markets` in the table's order: by exchange name, then pair, then
/// catalog id (the old app's sort).
pub(super) fn ordered(markets: &[MarketSelection]) -> Vec<Watched> {
    let mut watched: Vec<Watched> = markets
        .iter()
        .map(|market| Watched {
            key: PairKey::of(market),
            market: Rc::new(market.clone()),
        })
        .collect();
    watched.sort_by(|a, b| {
        let (a, b) = (&a.market, &b.market);
        format::compare_text(
            exchanges::display_name(&a.exchange),
            exchanges::display_name(&b.exchange),
        )
        .then_with(|| format::compare_text(&a.symbol, &b.symbol))
        .then_with(|| a.market_id.cmp(&b.market_id))
    });
    watched
}

/// Keeps one statistics watch per funding scope of the exchanges whose
/// markets are watched, and `feeds` listing them: an exchange with no
/// watched market is not polled, a scope no longer needed is let go (the
/// shared query keeps its value for a while, so watching it again soon is
/// instant), and a kept scope keeps its watch and its data. `offered` says
/// whether the capabilities offer any funding scope at all. Runs under the
/// session, so the watches live as long as it does.
fn follow_exchanges(
    session: SessionScope,
    ferris: Ferris,
    discovery: ReadSignal<QueryState<Capabilities>>,
    exchanges: Memo<Vec<String>>,
    feeds: Signal<Vec<Feed>>,
    offered: Signal<Option<bool>>,
) {
    let watches: Watches = Rc::default();
    let followed: RefCell<Option<(Rc<Capabilities>, Vec<String>)>> = RefCell::new(None);
    effect(move || {
        let Some(capabilities) = discovery.with(|state| state.data.clone()) else {
            return;
        };
        let wanted = exchanges.get();
        // The capabilities are refetched every few minutes; the same
        // document and the same exchanges change nothing.
        let same = followed
            .borrow()
            .as_ref()
            .is_some_and(|(document, exchanges)| {
                Rc::ptr_eq(document, &capabilities) && *exchanges == wanted
            });
        if same {
            return;
        }
        *followed.borrow_mut() = Some((capabilities.clone(), wanted.clone()));
        untracked(|| {
            let scopes = stats_scopes(&capabilities, &PRODUCTS, &FIELDS);
            let any = Some(!scopes.is_empty());
            if offered.get_untracked() != any {
                offered.set(any);
            }
            let polled = scopes
                .into_iter()
                .filter(|scope| wanted.contains(&scope.exchange));
            // A kept scope keeps its watch unless its cadence changed (the
            // new watch is made before the old one goes, so the value stays
            // cached between them).
            watches.follow(
                polled.map(|scope| (scope.key.clone(), Rc::new(scope))),
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

/// `market`'s statistics and the feed they came in: the last listing of
/// its catalog id among its exchange's scopes.
fn statistics<'a>(
    snaps: &'a [FeedSnap],
    market: &MarketSelection,
) -> Option<(&'a FeedSnap, &'a StatsMarket)> {
    let id = market.market_id.as_deref()?;
    snaps
        .iter()
        .filter(|snap| snap.scope.exchange == market.exchange)
        .filter_map(|snap| {
            let markets = &snap.snapshot.as_ref()?.markets;
            let stats = markets.iter().rev().find(|stats| stats.market_id == id)?;
            Some((snap, stats))
        })
        .next_back()
}

/// The rows of `watched`, in its order, from what `snaps` hold as of `now`.
pub(super) fn rows_of(watched: &[Watched], snaps: &[FeedSnap], now: f64) -> Vec<Row> {
    watched
        .iter()
        .map(|watched| row(watched, snaps, now))
        .collect()
}

/// `watched`'s row as of `now`: its rates while its funding is available
/// or stale, and its interval and age whatever its state (the old table's
/// rules).
fn row(watched: &Watched, snaps: &[FeedSnap], now: f64) -> Row {
    let field = statistics(snaps, &watched.market).and_then(|(snap, stats)| {
        let field = stats.fields.funding.as_ref()?;
        Some((snap.effective(Some(field.meta()), now), field))
    });
    let value = field.and_then(|(_, field)| field.value.as_ref());
    let equivalents = field
        .filter(|(state, _)| matches!(state, FieldState::Available | FieldState::Stale))
        .and(value)
        .and_then(|value| value.equivalents.as_ref());
    let rates = Period::ALL.map(|period| {
        let equivalents = equivalents?;
        let percent = match period {
            Period::Hour => &equivalents.one_hour_percent,
            Period::EightHours => &equivalents.eight_hour_percent,
            Period::Day => &equivalents.one_day_percent,
            Period::Year => &equivalents.annualized_percent,
        };
        percent.as_ref().map(Rate::of)
    });
    let received = field.and_then(|(_, field)| field.received_timestamp);
    Row {
        key: watched.key.clone(),
        market: watched.market.clone(),
        rates,
        interval: value.map_or_else(|| NONE.to_owned(), payment_interval),
        age: received.map_or_else(
            || NONE.to_owned(),
            |at| format!("{}s", ((now - at) / 1000.0).floor().max(0.0)),
        ),
        received: received.map_or_else(|| NONE.to_owned(), |at| format::iso_time(Some(at))),
    }
}

/// How often `funding` is paid: in whole hours or minutes when it is, else
/// in milliseconds.
fn payment_interval(funding: &FundingValue) -> String {
    const HOUR: f64 = 3_600_000.0;
    const MINUTE: f64 = 60_000.0;
    match funding.payment_interval_ms {
        None => NONE.to_owned(),
        Some(ms) if ms % HOUR == 0.0 => format!("{}h", ms / HOUR),
        Some(ms) if ms % MINUTE == 0.0 => format!("{}m", ms / MINUTE),
        Some(ms) => format!("{ms}ms"),
    }
}
