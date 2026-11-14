//! Candles: one market's OHLCV bars at one timeframe, shared by every
//! widget charting it (the old app's `FerrisOHLCVService`,
//! `OHLCVCacheService` and `ChartFeedSession`).
//!
//! A [`CandleFeed`] is a lease on the feed of one market and timeframe. The
//! first lease starts it: it subscribes to the market's `ohlcv` topic (the
//! timeframe among its params), then asks `fetchOHLCV` for the timeframe's
//! lookback (subscribing first, so the bar forming while the history is in
//! flight is not lost): the newest [`RECENT`] bars first, so a chart shows
//! at once, then the rest before them. Ferris caps a reply's bars per venue
//! and fills it from the start of the range asked for (1000 for Bybit and
//! Binance, 200 for KuCoin), so a range is asked for again from where each
//! reply stopped until it reaches its end ([`fetch_range`]); one request
//! would leave hours missing before the live bar. Later leases of the same market and timeframe share
//! it: one topic, one history, one buffer. The last lease dropping ends it.
//! The sharing, leases and listeners are the generic
//! [`FeedHub`](crate::data::net::FeedHub)'s; this module is the candle logic
//! only.
//!
//! The feed keeps the newest [`MAX_CANDLES`] bars in time order, one per
//! open time. A live bar replaces the kept bar of its time; a fetched bar
//! only fills a time nothing is kept for, since the kept one may be a newer
//! update of the bar still forming. [`CandleFeed::load_older`] fetches the
//! page before the oldest bar, one request at a time, until a page adds
//! nothing older.
//!
//! Each lease's `on_change` hears every change; it runs in the socket's
//! callback, so a consumer marks itself dirty there and does its work once
//! a frame.

use std::{cell::RefCell, rc::Rc};

use nsh_core::time::unix_ms;
use serde_json::{Value, json};

use super::{
    Availability, Channel, Ferris, FerrisPush, FerrisTopic, MarketKey, MarketSelection, Operation,
    Requirement, RestTarget, Venue,
};
use crate::data::net::{FeedLease, FeedNode, TaskHandle, TopicPhase};

/// The bars a feed keeps (the old chart's and cache's bound).
pub const MAX_CANDLES: usize = 25_000;
/// The bars one page of older history asks for (the old
/// `HISTORICAL_PAGE_SIZE`).
pub const HISTORY_PAGE: u32 = 500;
/// The newest bars a new feed asks for before the rest of its lookback.
const RECENT: u32 = 500;
/// The replies one range is asked for at most (the smallest cap seen, 200
/// bars, covers a 1600-bar lookback's rest in six).
const MAX_REPLIES: usize = 8;

/// A bar's length.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Timeframe {
    M1,
    M5,
    M15,
    H1,
    H4,
    D1,
    W1,
}

impl Timeframe {
    pub const ALL: [Self; 7] = [
        Self::M1,
        Self::M5,
        Self::M15,
        Self::H1,
        Self::H4,
        Self::D1,
        Self::W1,
    ];

    /// Ferris's name for it (`1m`), also what settings save.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::M1 => "1m",
            Self::M5 => "5m",
            Self::M15 => "15m",
            Self::H1 => "1h",
            Self::H4 => "4h",
            Self::D1 => "1d",
            Self::W1 => "1w",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|timeframe| timeframe.as_str() == name)
    }

    /// A bar's length in seconds.
    pub fn seconds(self) -> f64 {
        match self {
            Self::M1 => 60.0,
            Self::M5 => 300.0,
            Self::M15 => 900.0,
            Self::H1 => 3_600.0,
            Self::H4 => 14_400.0,
            Self::D1 => 86_400.0,
            Self::W1 => 604_800.0,
        }
    }

    /// The bars of history a new feed asks for (the old chart's
    /// `lookbackBars`).
    pub fn lookback(self) -> u32 {
        match self {
            Self::M1 => 1_600,
            Self::M5 => 1_500,
            Self::M15 => 1_400,
            Self::H1 => 1_200,
            Self::H4 => 1_000,
            Self::D1 => 720,
            Self::W1 => 520,
        }
    }
}

/// One bar, as Ferris's unified candle shape gives it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candle {
    /// When the bar opened, in whole Unix seconds.
    pub time: f64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    /// What traded in it, when the venue says.
    pub volume: Option<f64>,
}

impl Candle {
    /// Reads one bar as the old app did: `[time, open, high, low, close,
    /// volume]`, or an object with those members (`timestamp`, `time` or
    /// `t`; `open` or `o`; ...), numbers or numeric text. The time is
    /// seconds or milliseconds; the high and low are widened to hold the
    /// open and close. `None` without a time or any of the four prices.
    pub fn from_value(value: &Value) -> Option<Self> {
        let number = |value: Option<&Value>| {
            match value? {
                Value::Number(number) => number.as_f64(),
                Value::String(text) => text.trim().parse().ok(),
                _ => None,
            }
            .filter(|number: &f64| number.is_finite())
        };
        let [time, open, high, low, close, volume] = match value {
            Value::Array(row) => [0, 1, 2, 3, 4, 5].map(|at| number(row.get(at))),
            Value::Object(bar) => [
                ["timestamp", "time", "t"].as_slice(),
                &["open", "o"],
                &["high", "h"],
                &["low", "l"],
                &["close", "c"],
                &["volume", "v"],
            ]
            .map(|names| names.iter().find_map(|name| number(bar.get(*name)))),
            _ => return None,
        };
        let (time, open, high, low, close) = (time?, open?, high?, low?, close?);
        let seconds = if time > 1e12 { time / 1_000.0 } else { time };
        Some(Self {
            time: seconds.floor(),
            open,
            high: high.max(open).max(close).max(low),
            low: low.min(open).min(close).min(high),
            close,
            volume,
        })
    }
}

/// The bars in a payload: an array, or one under `ohlcv`, `data` or
/// `result`, at the top or inside `data`.
fn bars_in(payload: &Value) -> &[Value] {
    fn list(value: &Value) -> Option<&Vec<Value>> {
        match value {
            Value::Array(list) => Some(list),
            Value::Object(record) => ["ohlcv", "data", "result"]
                .iter()
                .find_map(|name| record.get(*name)?.as_array()),
            _ => None,
        }
    }
    list(payload)
        .or_else(|| list(payload.get("data")?))
        .map_or(&[], Vec::as_slice)
}

/// A payload's bars, oldest first, one per time (the last one given).
pub fn decode(payload: &Value) -> Vec<Candle> {
    let mut bars: Vec<Candle> = bars_in(payload)
        .iter()
        .filter_map(Candle::from_value)
        .collect();
    bars.sort_by(|a, b| a.time.total_cmp(&b.time));
    bars.dedup_by(|later, kept| {
        let repeat = later.time == kept.time;
        if repeat {
            *kept = *later;
        }
        repeat
    });
    bars
}

/// Whether a page of older bars is wanted, on its way, or pointless.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Older {
    Idle,
    Loading,
    /// A page added nothing older (the venue's first bar, or the feed is
    /// full).
    Exhausted,
}

/// A feed's buffer and status.
struct State {
    /// The newest bars, oldest first, one per time.
    candles: Vec<Candle>,
    /// How many bars were added after the newest one so far.
    appended: u64,
    /// The first history has not answered yet.
    loading: bool,
    /// The first history could not be fetched.
    history_failed: bool,
    /// The server refused the topic, or it could not be made.
    failed: bool,
    older: Older,
}

impl State {
    /// Takes a live bar: it replaces the kept bar of its time, else goes in
    /// time order. True when anything changed.
    fn upsert(&mut self, candle: Candle) -> bool {
        let at = self.candles.partition_point(|kept| kept.time < candle.time);
        match self.candles.get_mut(at) {
            Some(kept) if kept.time == candle.time => {
                if *kept == candle {
                    return false;
                }
                *kept = candle;
            }
            _ => {
                if at == self.candles.len() {
                    self.appended += 1;
                }
                self.candles.insert(at, candle);
                self.trim();
            }
        }
        true
    }

    /// Takes fetched bars (oldest first, one per time): each fills a time
    /// nothing is kept for. True when any was added.
    fn fill(&mut self, fetched: Vec<Candle>) -> bool {
        let before = self.candles.len();
        let had_bars = before > 0;
        let mut kept = std::mem::take(&mut self.candles).into_iter().peekable();
        let mut fetched = fetched.into_iter().peekable();
        let mut merged = Vec::with_capacity(before + fetched.len());
        loop {
            match (kept.peek(), fetched.peek()) {
                (Some(bar), Some(new)) if new.time < bar.time => merged.extend(fetched.next()),
                (Some(bar), Some(new)) if new.time == bar.time => drop(fetched.next()),
                (Some(_), _) => merged.extend(kept.next()),
                (None, Some(_)) => {
                    self.appended += u64::from(had_bars);
                    merged.extend(fetched.next());
                }
                (None, None) => break,
            }
        }
        let added = merged.len() > before;
        self.candles = merged;
        self.trim();
        added
    }

    /// Drops the oldest bars beyond [`MAX_CANDLES`].
    fn trim(&mut self) {
        let extra = self.candles.len().saturating_sub(MAX_CANDLES);
        self.candles.drain(..extra);
    }
}

/// A market and timeframe's feed, shared by its leases.
type Feed = FeedNode<(MarketKey, Timeframe), Bars>;

/// What a feed keeps: its market and timeframe, its buffer and status, and
/// what streams into them.
struct Bars {
    market: MarketSelection,
    timeframe: Timeframe,
    state: RefCell<State>,
    subscription: RefCell<Option<super::Subscription>>,
    /// The first history's request.
    history: RefCell<Option<TaskHandle>>,
    /// The request for the page before the oldest bar.
    older: RefCell<Option<TaskHandle>>,
}

impl Bars {
    fn new(market: &MarketSelection, timeframe: Timeframe) -> Self {
        Self {
            market: market.clone(),
            timeframe,
            state: RefCell::new(State {
                candles: Vec::new(),
                appended: 0,
                loading: true,
                history_failed: false,
                failed: false,
                older: Older::Idle,
            }),
            subscription: RefCell::new(None),
            history: RefCell::new(None),
            older: RefCell::new(None),
        }
    }

    fn label(&self) -> String {
        format!(
            "{} {} {}",
            self.market.exchange,
            self.market.request_symbol,
            self.timeframe.as_str()
        )
    }
}

/// The market's `ohlcv` topic at `timeframe`.
fn topic(
    market: &MarketSelection,
    timeframe: Timeframe,
) -> Result<FerrisTopic, crate::data::net::ApiError> {
    let mut topic = market.topic(Channel::Ohlcv)?;
    if let FerrisTopic::Market { params, .. } = &mut topic {
        params.insert("timeframe".to_owned(), json!(timeframe.as_str()));
    }
    Ok(topic)
}

/// Starts a new feed: subscribes to its topic, then asks for its history.
fn start(ferris: &Ferris, feed: &Rc<Feed>) {
    let bars = feed.state();
    match topic(&bars.market, bars.timeframe) {
        Ok(topic) => {
            let (on_data, on_phase) = (Rc::downgrade(feed), Rc::downgrade(feed));
            let subscription = ferris.subscribe_with_phase(
                topic,
                move |push: &FerrisPush| {
                    if let Some(feed) = on_data.upgrade() {
                        receive(&feed, push.data());
                    }
                },
                move |phase| {
                    if let Some(feed) = on_phase.upgrade() {
                        phase_changed(&feed, phase);
                    }
                },
            );
            *bars.subscription.borrow_mut() = Some(subscription);
        }
        Err(error) => {
            log::error!("Candles: cannot stream {}: {}", bars.label(), error.message);
            bars.state.borrow_mut().failed = true;
        }
    }
    let (ferris, weak) = (ferris.clone(), Rc::downgrade(feed));
    let (market, timeframe, label) = (bars.market.clone(), bars.timeframe, bars.label());
    let history = TaskHandle::spawn(async move {
        let now = (unix_ms() / 1_000.0).floor();
        let step = timeframe.seconds();
        let lookback = timeframe.lookback();
        let recent = now - step * f64::from(lookback.min(RECENT));
        let oldest = now - step * f64::from(lookback);
        // The newest bars, then the rest of the lookback before them.
        for (since, until) in [(recent, now + step), (oldest, recent - 1.0)] {
            if until < since {
                break;
            }
            let fetched = fetch_range(&ferris, &market, timeframe, since, until, &label).await;
            let Some(feed) = weak.upgrade() else {
                return;
            };
            let failed = fetched.is_none();
            {
                let mut state = feed.state().state.borrow_mut();
                match fetched {
                    Some(bars) => {
                        state.fill(bars);
                    }
                    // Only the newest bars' failure is the chart's to say.
                    None => state.history_failed |= state.loading,
                }
                state.loading = false;
            }
            feed.notify();
            if failed {
                break;
            }
        }
    });
    *bars.history.borrow_mut() = Some(history);
}

/// Takes in a live frame's bars.
fn receive(feed: &Feed, payload: &Value) {
    let changed = {
        let mut state = feed.state().state.borrow_mut();
        // Live data says the topic streams.
        let mut changed = std::mem::replace(&mut state.failed, false);
        for candle in decode(payload) {
            changed |= state.upsert(candle);
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
        let failed = match phase {
            TopicPhase::Failed => true,
            TopicPhase::Subscribed | TopicPhase::Live => false,
            _ => state.failed,
        };
        std::mem::replace(&mut state.failed, failed) != failed
    };
    if changed {
        feed.notify();
    }
}

/// What a `fetchOHLCV` for `market` names: its REST target, except on
/// venues whose candles are told apart by catalog id and product type
/// ([`Venue::candles_by_type`]).
fn rest_target(market: &MarketSelection) -> Option<RestTarget> {
    let mut target = market.rest_target().ok()?;
    if Venue::of(&market.exchange).candles_by_type {
        target.symbol = market
            .market_id
            .clone()
            .unwrap_or_else(|| market.symbol.clone());
        target.params = [("type".to_owned(), json!(market.kind.as_str()))]
            .into_iter()
            .collect();
    }
    Some(target)
}

/// `market`'s bars that opened from `since` to `until` (Unix seconds),
/// oldest first, whatever a reply's cap: while a reply stops short of
/// `until`, the range is asked for again from the bar after its newest (at
/// most [`MAX_REPLIES`] times). `None` when the first request fails; a
/// later failure keeps what came before it.
async fn fetch_range(
    ferris: &Ferris,
    market: &MarketSelection,
    timeframe: Timeframe,
    since: f64,
    until: f64,
    label: &str,
) -> Option<Vec<Candle>> {
    let step = timeframe.seconds();
    let mut bars: Vec<Candle> = Vec::new();
    let mut from = since;
    for _ in 0..MAX_REPLIES {
        let Some(reply) = fetch(ferris, market, timeframe, from, until, label).await else {
            return (!bars.is_empty()).then_some(bars);
        };
        let Some(newest) = reply.last().map(|bar| bar.time) else {
            break;
        };
        bars.extend(reply);
        // The forming bar may not be served yet, nor a quiet minute's.
        if newest >= until - 3.0 * step {
            break;
        }
        from = newest + step;
    }
    Some(bars)
}

/// One reply's bars of `market` that opened from `since` to `until` (Unix
/// seconds), oldest first. `None` when its venue publishes no candle
/// history or the request fails, which is logged.
async fn fetch(
    ferris: &Ferris,
    market: &MarketSelection,
    timeframe: Timeframe,
    since: f64,
    until: f64,
    label: &str,
) -> Option<Vec<Candle>> {
    if let Ok(capabilities) = ferris.capabilities().await
        && Requirement::snapshot(Operation::FetchOhlcv).availability(
            &capabilities,
            &market.exchange,
            market.kind.product(),
        ) == Availability::Unsupported
    {
        return None;
    }
    let RestTarget {
        exchange,
        symbol,
        params,
    } = rest_target(market)?;
    // `since` and `until` are request bounds, not market params: venues
    // such as Phoenix reject them inside `params`.
    let wanted = ((until - since) / timeframe.seconds()).ceil() + 10.0;
    let body = json!({
        "exchange": exchange,
        "symbol": symbol,
        "timeframe": timeframe.as_str(),
        "since": (since * 1_000.0) as i64,
        "until": (until * 1_000.0) as i64,
        "limit": wanted.clamp(50.0, 5_000.0) as u32,
        "params": params,
    });
    match ferris.request::<Value>(Operation::FetchOhlcv, &body).await {
        Ok(payload) => Some(
            decode(&payload)
                .into_iter()
                .filter(|bar| bar.time >= since && bar.time <= until)
                .collect(),
        ),
        Err(error) => {
            log::warn!(
                "Candles: no history for {label} ({}: {})",
                error.code,
                error.message
            );
            None
        }
    }
}

impl Ferris {
    /// A lease on `market`'s candles at `timeframe`: its shared feed (by
    /// [`MarketSelection::identity`] and timeframe), started if no lease
    /// holds it. `on_change` is told each time the feed's bars or status
    /// change, until the lease drops.
    pub fn candles(
        &self,
        market: &MarketSelection,
        timeframe: Timeframe,
        on_change: impl Fn() + 'static,
    ) -> CandleFeed {
        let lease = self.feeds.hub::<(MarketKey, Timeframe), Bars>().lease(
            (market.identity(), timeframe),
            || Bars::new(market, timeframe),
            |feed| start(self, feed),
            on_change,
        );
        CandleFeed {
            lease,
            ferris: self.clone(),
        }
    }
}

/// One consumer's lease on a market's candles (see [`Ferris::candles`]).
/// Dropping it stops its `on_change`, and the last lease ends the feed.
#[must_use = "dropping a candle feed releases it"]
pub struct CandleFeed {
    lease: FeedLease<(MarketKey, Timeframe), Bars>,
    ferris: Ferris,
}

impl CandleFeed {
    fn state(&self) -> std::cell::Ref<'_, State> {
        self.lease.state().state.borrow()
    }

    /// The bars kept, oldest first.
    pub fn with_candles<R>(&self, f: impl FnOnce(&[Candle]) -> R) -> R {
        f(&self.state().candles)
    }

    /// How many bars were ever added after the newest: what a view looking
    /// at older bars moves by to stay on them.
    pub fn appended(&self) -> u64 {
        self.state().appended
    }

    /// Waiting for its first history.
    pub fn loading(&self) -> bool {
        self.state().loading
    }

    /// Its first history could not be fetched; live bars still arrive.
    pub fn history_failed(&self) -> bool {
        self.state().history_failed
    }

    /// The server refused its topic, or it could not be asked for.
    pub fn failed(&self) -> bool {
        self.state().failed
    }

    /// A page of older bars is on its way.
    pub fn loading_older(&self) -> bool {
        self.state().older == Older::Loading
    }

    /// Asks for the page of bars before the oldest kept; `on_change` hears
    /// when it has answered. Does nothing (false) while the first history
    /// or another page is on its way, with no bar kept, or once a page
    /// added nothing older. A failed page can be asked for again.
    pub fn load_older(&self) -> bool {
        let feed = self.lease.node();
        let bars = feed.state();
        let oldest = {
            let mut state = bars.state.borrow_mut();
            let Some(oldest) = state.candles.first().map(|bar| bar.time) else {
                return false;
            };
            if state.loading || state.older != Older::Idle {
                return false;
            }
            state.older = Older::Loading;
            oldest
        };
        let (ferris, weak) = (self.ferris.clone(), Rc::downgrade(feed));
        let (market, timeframe, label) = (bars.market.clone(), bars.timeframe, bars.label());
        let page = TaskHandle::spawn(async move {
            let until = oldest - 1.0;
            let since = until - timeframe.seconds() * f64::from(HISTORY_PAGE);
            let fetched = fetch_range(&ferris, &market, timeframe, since, until, &label).await;
            let Some(feed) = weak.upgrade() else {
                return;
            };
            {
                let mut state = feed.state().state.borrow_mut();
                state.older = match fetched {
                    None => Older::Idle,
                    Some(bars) => {
                        state.fill(bars);
                        let older = state.candles.first().is_some_and(|bar| bar.time < oldest);
                        if older { Older::Idle } else { Older::Exhausted }
                    }
                };
            }
            feed.notify();
        });
        *bars.older.borrow_mut() = Some(page);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(time: f64, close: f64) -> Candle {
        Candle {
            time,
            open: close,
            high: close,
            low: close,
            close,
            volume: None,
        }
    }

    fn state(candles: Vec<Candle>) -> State {
        State {
            candles,
            appended: 0,
            loading: false,
            history_failed: false,
            failed: false,
            older: Older::Idle,
        }
    }

    #[test]
    fn bars_read_from_rows_and_objects() {
        let row = json!([
            1_794_626_580_000_i64,
            "67405.9",
            67400.0,
            67353.7,
            67359.6,
            21.7
        ]);
        let candle = Candle::from_value(&row).expect("a row is a bar");
        assert_eq!(candle.time, 1_794_626_580.0);
        // The high is widened to hold the open.
        assert_eq!((candle.open, candle.high), (67405.9, 67405.9));
        assert_eq!(candle.volume, Some(21.7));

        let object = json!({"t": 1_794_626_580, "o": 1, "h": 3, "l": 0.5, "c": 2});
        let candle = Candle::from_value(&object).expect("an object is a bar");
        assert_eq!(
            (candle.time, candle.low, candle.close),
            (1_794_626_580.0, 0.5, 2.0)
        );
        assert_eq!(candle.volume, None);

        assert_eq!(Candle::from_value(&json!([1, 2, 3, 4])), None);
        assert_eq!(Candle::from_value(&json!([1, 2, "", 4, 5])), None);
    }

    #[test]
    fn payloads_decode_sorted_one_bar_per_time() {
        let payload =
            json!({"data": {"ohlcv": [[120, 1, 1, 1, 1], [60, 2, 2, 2, 2], [120, 3, 3, 3, 3]]}});
        let bars = decode(&payload);
        assert_eq!(bars.len(), 2);
        assert_eq!((bars[0].time, bars[0].close), (60.0, 2.0));
        assert_eq!((bars[1].time, bars[1].close), (120.0, 3.0));
        assert!(decode(&json!({"data": "none"})).is_empty());
    }

    #[test]
    fn live_bars_replace_their_time_and_history_only_fills() {
        let mut state = state(vec![bar(120.0, 5.0)]);
        // The forming bar updates in place; a new bar is appended.
        assert!(state.upsert(bar(120.0, 6.0)));
        assert!(!state.upsert(bar(120.0, 6.0)));
        assert!(state.upsert(bar(180.0, 7.0)));
        assert_eq!(state.appended, 1);
        // History fills the times before and between, never a kept one.
        assert!(state.fill(vec![bar(60.0, 1.0), bar(120.0, 1.0), bar(150.0, 2.0)]));
        let closes: Vec<_> = state
            .candles
            .iter()
            .map(|bar| (bar.time, bar.close))
            .collect();
        assert_eq!(
            closes,
            [(60.0, 1.0), (120.0, 6.0), (150.0, 2.0), (180.0, 7.0)]
        );
        assert_eq!(state.appended, 1);
        assert!(!state.fill(vec![bar(60.0, 9.0)]));
        // A bar after the newest counts as appended.
        assert!(state.fill(vec![bar(240.0, 8.0)]));
        assert_eq!(state.appended, 2);
    }

    #[test]
    fn the_oldest_bars_go_beyond_the_bound() {
        let mut state = state((0..MAX_CANDLES).map(|at| bar(at as f64, 1.0)).collect());
        assert!(state.upsert(bar(MAX_CANDLES as f64, 2.0)));
        assert_eq!(state.candles.len(), MAX_CANDLES);
        assert_eq!(state.candles[0].time, 1.0);
    }
}
