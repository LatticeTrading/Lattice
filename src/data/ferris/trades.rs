//! Trades: one market's recent prints, shared by every widget watching it
//! (the old app's `FerrisTradesService` and `SharedTradeFeed`).
//!
//! A [`TradeFeed`] is a lease on one market's feed. The first lease of a
//! market starts it: it subscribes to the market's `trades` topic, then
//! asks `fetchTrades` for the last 200 prints (subscribing first, so no
//! print is lost while the snapshot is in flight; venues that publish no
//! trade history, Perpl and Variational today, are never asked). Later
//! leases of the same market share it: one topic, one snapshot, one
//! buffer. The last lease dropping ends it (the socket unsubscribes after
//! its grace). The sharing, leases and listeners are the generic
//! [`FeedHub`](crate::data::net::FeedHub)'s; this module is the trade
//! logic only.
//!
//! The feed keeps the newest [`MAX_ROWS`] prints in time order, drops
//! prints it has seen (by id, the last [`MAX_SEEN`]), and counts live
//! arrivals over the last five seconds for the trades-per-second rate.
//! Ids alone cannot drop the prints the snapshot and the stream both carry
//! around the moment the feed starts: some venues number a print
//! differently in each. So the two meet at a [`Seam`] in time, and each
//! side only adds the prints on its side of it.
//! Each lease's `on_change` hears every change; it runs in the socket's
//! callback, so a consumer marks itself dirty there and does its work once
//! a frame.

use std::{
    cell::RefCell,
    collections::{HashSet, VecDeque},
    rc::Rc,
    time::Duration,
};

use nsh_core::{prelude::*, time::unix_ms};
use serde_json::{Value, json};

use super::{
    Availability, Channel, Ferris, FerrisPush, FerrisTopic, MarketKey, MarketSelection, Operation,
};
use crate::data::net::{FeedLease, FeedNode, TaskHandle, TopicPhase};

/// The prints a feed keeps, and the snapshot it asks for.
pub const MAX_ROWS: usize = 200;
/// The ids a feed remembers to drop repeats.
const MAX_SEEN: usize = 5_000;
/// The window the trades-per-second rate counts arrivals over.
const RATE_WINDOW_MS: f64 = 5_000.0;
/// Arrivals kept for the rate, at most (a burst past this is undercounted).
const MAX_ARRIVALS: usize = MAX_SEEN * 2;
/// How long a feed reads as loading with no snapshot and no print.
const LOADING_FOR: Duration = Duration::from_secs(5);

/// Which side took liquidity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Buy,
    Sell,
}

/// One print, as Ferris's unified trade shape gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct Trade {
    /// The venue's id (`info.tid` when the unified id is missing; else made
    /// from the time, price and amount).
    pub id: String,
    pub side: Side,
    pub price: f64,
    pub amount: f64,
    /// When it printed, in Unix milliseconds.
    pub timestamp: f64,
    /// A forced close (`"type":"LIQUIDATION"`), not an ordinary print.
    pub liquidation: bool,
}

impl Trade {
    /// Reads one unified trade as the old app did: a missing or unreadable
    /// time is `now`, a missing or unreadable price or amount is zero.
    /// `None` only when it is not an object.
    pub fn from_value(value: &Value, now: f64) -> Option<Self> {
        let trade = value.as_object()?;
        let number = |value: Option<&Value>| {
            match value? {
                Value::Number(number) => number.as_f64(),
                Value::String(text) => text.trim().parse().ok(),
                _ => None,
            }
            .filter(|number: &f64| number.is_finite())
        };
        let id_text = |value: Option<&Value>| match value? {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        };
        let timestamp = number(trade.get("timestamp")).unwrap_or(now);
        let price = number(trade.get("price")).unwrap_or(0.0);
        let amount = number(trade.get("amount")).unwrap_or(0.0);
        let id = id_text(trade.get("id"))
            .or_else(|| id_text(trade.get("info").and_then(|info| info.get("tid"))))
            .unwrap_or_else(|| format!("{timestamp}-{price}-{amount}"));
        Some(Self {
            id,
            side: match trade.get("side").and_then(Value::as_str) {
                Some("sell") => Side::Sell,
                _ => Side::Buy,
            },
            price,
            amount,
            timestamp,
            liquidation: trade.get("type").and_then(Value::as_str) == Some("LIQUIDATION"),
        })
    }

    /// What tells it from every other print of its market.
    fn identity(&self) -> String {
        let kind = if self.liquidation {
            "liquidation"
        } else {
            "trade"
        };
        format!("{kind}:{}", self.id)
    }
}

/// The trades in a payload: an array, or one under `trades` or `data`.
fn trades_in(payload: &Value) -> &[Value] {
    let list = match payload {
        Value::Object(record) => record.get("trades").or_else(|| record.get("data")),
        other => Some(other),
    };
    list.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// When the oldest and the newest of `trades` printed; `None` for none.
fn span(trades: &[Trade]) -> Option<(f64, f64)> {
    let mut times = trades.iter().map(|trade| trade.timestamp);
    let first = times.next()?;
    Some(times.fold((first, first), |(oldest, newest), at| {
        (oldest.min(at), newest.max(at))
    }))
}

/// A print a feed kept, numbered in arrival order (what keys it in a list).
#[derive(Debug, PartialEq)]
pub struct TradeRow {
    pub trade: Trade,
    pub sequence: u64,
}

/// Where the snapshot meets the live stream.
///
/// Both carry the prints of the moment the feed starts (it subscribes,
/// then fetches), and ids cannot always tell they are the same prints: on
/// Ferris, Binance's snapshot is aggregate trades and its stream raw ones,
/// Aster's the reverse, and Phoenix numbers a snapshot print by its
/// transaction signature and a live one by a sequence number (checked live
/// 2026-11-13; Bybit, Hyperliquid, Apex, KuCoin and Bitfinex share ids).
/// So whichever side brings prints first keeps them, and the other only
/// adds the prints beyond it: a snapshot after the stream adds what is
/// older than the first live print, a stream after the snapshot adds what
/// is newer than its newest. Ids still drop repeats within each side.
#[derive(Clone, Copy, Debug)]
enum Seam {
    /// Neither side has brought a print yet.
    Open,
    /// Live prints came first; the oldest of the first frame printed then.
    LiveFrom(f64),
    /// The snapshot came first; its newest print printed then.
    SnapshotTo(f64),
}

impl Seam {
    /// Whether a print at `timestamp` from the stream (`live`) or the
    /// snapshot is beyond what the other side already brought.
    fn admits(self, timestamp: f64, live: bool) -> bool {
        match (self, live) {
            (Self::LiveFrom(from), false) => timestamp < from,
            (Self::SnapshotTo(to), true) => timestamp > to,
            _ => true,
        }
    }

    /// The seam once a side brought prints printed `oldest` to `newest`:
    /// set by the first side to bring any, then fixed.
    fn after(self, live: bool, oldest: f64, newest: f64) -> Self {
        match self {
            Self::Open if live => Self::LiveFrom(oldest),
            Self::Open => Self::SnapshotTo(newest),
            fixed => fixed,
        }
    }
}

/// A feed's buffer and status.
struct State {
    /// The newest prints, oldest first.
    rows: VecDeque<Rc<TradeRow>>,
    seen: HashSet<String>,
    seen_order: VecDeque<String>,
    /// When each live print arrived (Unix ms), oldest first.
    arrivals: VecDeque<f64>,
    sequence: u64,
    seam: Seam,
    /// No snapshot and no print yet, for at most [`LOADING_FOR`].
    loading: bool,
    /// The server refused the topic, or it could not be made.
    failed: bool,
}

impl State {
    fn prune_arrivals(&mut self, now: f64) {
        while self
            .arrivals
            .front()
            .is_some_and(|&at| at <= now - RATE_WINDOW_MS)
        {
            self.arrivals.pop_front();
        }
        while self.arrivals.len() > MAX_ARRIVALS {
            self.arrivals.pop_front();
        }
    }

    /// Keeps `trade` unless it was seen; true when kept.
    fn keep(&mut self, trade: Trade) -> bool {
        let identity = trade.identity();
        if !self.seen.insert(identity.clone()) {
            return false;
        }
        self.seen_order.push_back(identity);
        while self.seen_order.len() > MAX_SEEN {
            if let Some(stale) = self.seen_order.pop_front() {
                self.seen.remove(&stale);
            }
        }
        let row = Rc::new(TradeRow {
            trade,
            sequence: self.sequence,
        });
        self.sequence += 1;
        // In time order; a live print almost always goes at the end.
        let at = self
            .rows
            .iter()
            .rposition(|kept| kept.trade.timestamp <= row.trade.timestamp)
            .map_or(0, |before| before + 1);
        self.rows.insert(at, row);
        if self.rows.len() > MAX_ROWS {
            self.rows.pop_front();
        }
        true
    }
}

/// A market's feed, shared by its leases.
type Feed = FeedNode<MarketKey, Prints>;

/// What a market's feed keeps: the market, its buffer and status, and what
/// streams into them.
struct Prints {
    market: MarketSelection,
    state: RefCell<State>,
    subscription: RefCell<Option<super::Subscription>>,
    /// The snapshot and the loading deadline.
    tasks: RefCell<Vec<TaskHandle>>,
}

impl Prints {
    fn new(market: &MarketSelection) -> Self {
        Self {
            market: market.clone(),
            state: RefCell::new(State {
                rows: VecDeque::new(),
                seen: HashSet::new(),
                seen_order: VecDeque::new(),
                arrivals: VecDeque::new(),
                sequence: 0,
                seam: Seam::Open,
                loading: true,
                failed: false,
            }),
            subscription: RefCell::new(None),
            tasks: RefCell::new(Vec::new()),
        }
    }
}

/// Starts a new feed: subscribes to its market's topic, asks for the
/// snapshot, and sets the loading deadline.
fn start(ferris: &Ferris, feed: &Rc<Feed>) {
    let market = &feed.state().market;
    let label = format!("{} {}", market.exchange, market.request_symbol);
    match market.topic(Channel::Trades) {
        Ok(topic) => {
            let (on_data, on_phase) = (Rc::downgrade(feed), Rc::downgrade(feed));
            let subscription = ferris.subscribe_with_phase(
                topic,
                move |push: &FerrisPush| {
                    if let Some(feed) = on_data.upgrade() {
                        receive(&feed, push.data(), true);
                    }
                },
                move |phase| {
                    if let Some(feed) = on_phase.upgrade() {
                        phase_changed(&feed, phase);
                    }
                },
            );
            *feed.state().subscription.borrow_mut() = Some(subscription);
        }
        Err(error) => {
            log::error!("Trades: cannot stream {label}: {}", error.message);
            let mut state = feed.state().state.borrow_mut();
            state.loading = false;
            state.failed = true;
        }
    }
    let snapshot = {
        let (ferris, weak, market) = (ferris.clone(), Rc::downgrade(feed), market.clone());
        TaskHandle::spawn(async move {
            let Some(payload) = snapshot(&ferris, &market, &label).await else {
                if let Some(feed) = weak.upgrade() {
                    settle(&feed);
                }
                return;
            };
            if let Some(feed) = weak.upgrade() {
                receive(&feed, &payload, false);
            }
        })
    };
    let deadline = {
        let weak = Rc::downgrade(feed);
        TaskHandle::spawn(async move {
            timer(LOADING_FOR).await;
            if let Some(feed) = weak.upgrade() {
                settle(&feed);
            }
        })
    };
    feed.state().tasks.borrow_mut().extend([snapshot, deadline]);
}

/// Takes in a snapshot (`live: false`) or a live frame's prints, the ones
/// beyond the [`Seam`].
fn receive(feed: &Feed, payload: &Value, live: bool) {
    let now = unix_ms();
    let changed = {
        let mut state = feed.state().state.borrow_mut();
        // Only live data says the topic streams; a snapshot does not.
        let mut changed = state.loading || (live && state.failed);
        state.loading = false;
        if live {
            state.failed = false;
        }
        let trades: Vec<Trade> = trades_in(payload)
            .iter()
            .filter_map(|trade| Trade::from_value(trade, now))
            .collect();
        let seam = state.seam;
        if let Some((oldest, newest)) = span(&trades) {
            state.seam = seam.after(live, oldest, newest);
        }
        for trade in trades
            .into_iter()
            .filter(|trade| seam.admits(trade.timestamp, live))
        {
            if state.keep(trade) {
                changed = true;
                if live {
                    state.arrivals.push_back(now);
                }
            }
        }
        if live {
            state.prune_arrivals(now);
        }
        changed
    };
    if changed {
        feed.notify();
    }
}

fn phase_changed(feed: &Feed, phase: TopicPhase) {
    let changed = {
        let mut state = feed.state().state.borrow_mut();
        match phase {
            TopicPhase::Failed if !state.failed => {
                state.failed = true;
                state.loading = false;
                true
            }
            TopicPhase::Subscribed | TopicPhase::Live if state.failed => {
                state.failed = false;
                true
            }
            _ => false,
        }
    };
    if changed {
        feed.notify();
    }
}

/// No longer loading (the snapshot failed or the deadline passed), whether
/// or not a print came.
fn settle(feed: &Feed) {
    let changed = std::mem::replace(&mut feed.state().state.borrow_mut().loading, false);
    if changed {
        feed.notify();
    }
}

/// The last [`MAX_ROWS`] prints of `market`, unless its venue publishes no
/// trade history. A failure is logged and the feed goes on live only, as
/// the old app's did.
async fn snapshot(ferris: &Ferris, market: &MarketSelection, label: &str) -> Option<Value> {
    if let Ok(capabilities) = ferris.capabilities().await
        && capabilities.snapshot(
            &market.exchange,
            Operation::FetchTrades,
            Some(market.kind.product()),
        ) == Availability::Unsupported
    {
        return None;
    }
    let FerrisTopic::Market {
        exchange,
        symbol,
        params,
        ..
    } = market.rest_topic(Channel::Trades).ok()?
    else {
        return None;
    };
    let body = json!({
        "exchange": exchange,
        "symbol": symbol,
        "params": params,
        "limit": MAX_ROWS,
    });
    match ferris.request::<Value>(Operation::FetchTrades, &body).await {
        Ok(payload) => Some(payload),
        Err(error) => {
            log::warn!(
                "Trades: no snapshot for {label} ({}: {}); streaming only",
                error.code,
                error.message
            );
            None
        }
    }
}

impl Ferris {
    /// A lease on `market`'s trades: its shared feed (by
    /// [`MarketSelection::identity`]), started if no lease holds it.
    /// `on_change` is told each time the feed's prints or status change,
    /// until the lease drops.
    pub fn trades(&self, market: &MarketSelection, on_change: impl Fn() + 'static) -> TradeFeed {
        let lease = self.feeds.hub::<MarketKey, Prints>().lease(
            market.identity(),
            || Prints::new(market),
            |feed| start(self, feed),
            on_change,
        );
        TradeFeed(lease)
    }
}

/// One consumer's lease on a market's trades (see [`Ferris::trades`]).
/// Dropping it stops its `on_change`, and the last lease ends the feed.
#[must_use = "dropping a trade feed releases it"]
pub struct TradeFeed(FeedLease<MarketKey, Prints>);

impl TradeFeed {
    /// The market, as the first lease selected it.
    pub fn market(&self) -> &MarketSelection {
        &self.0.state().market
    }

    /// The prints kept, oldest first.
    pub fn with_rows<R>(&self, f: impl FnOnce(&VecDeque<Rc<TradeRow>>) -> R) -> R {
        f(&self.0.state().state.borrow().rows)
    }

    /// Waiting for its first snapshot or print (at most a few seconds).
    pub fn loading(&self) -> bool {
        self.0.state().state.borrow().loading
    }

    /// The server refused its topic, or it could not be asked for.
    pub fn failed(&self) -> bool {
        self.0.state().state.borrow().failed
    }

    /// Live prints per second over the last five seconds.
    pub fn trades_per_second(&self, now: f64) -> f64 {
        let mut state = self.0.state().state.borrow_mut();
        state.prune_arrivals(now);
        state.arrivals.len() as f64 / (RATE_WINDOW_MS / 1_000.0)
    }
}
