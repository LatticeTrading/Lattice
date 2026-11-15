//! Order books: one market's bids and asks at a depth, shared by every
//! widget showing them (the old app's `FerrisOrderBookService` and
//! `OrderBookDataService`).
//!
//! An [`OrderBookFeed`] is a lease on the feed of one market at one depth.
//! The first lease starts it: it subscribes to the market's `orderbook`
//! topic (the depth among its params, as `levels`), then asks
//! `fetchOrderBook` for the book as it stands, so a quiet market shows at
//! once. Every live frame is the whole book at that depth, so the newest
//! one replaces what is kept; the snapshot only counts while no live frame
//! has come (it is the older of the two once one has). Later leases of the
//! same market and depth share it: one topic, one snapshot, one book. The
//! last lease dropping ends it. The sharing, leases and listeners are the
//! generic [`FeedHub`](crate::data::net::FeedHub)'s; this module is the
//! book logic only.
//!
//! A depth is what a consumer asks for; the levels a feed streams are what
//! its venue serves for it ([`stream_levels`]): Bybit's books come in 1,
//! 50, 200 and 1000 levels, KuCoin's and Bitfinex's hold at most 100 a
//! side. Two depths that stream the same levels share one feed.
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
use crate::data::net::{ApiError, FeedLease, FeedNode, TaskHandle, TopicPhase};

/// One price level of a side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BookLevel {
    pub price: f64,
    /// What rests at the price, in the venue's unit (the base asset; on
    /// KuCoin's contracts, contracts).
    pub amount: f64,
    /// Price times amount.
    pub total: f64,
    /// The amounts from the best price of its side through this one.
    pub cumulative: f64,
}

/// One market's book: both sides from their best price outward.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OrderBook {
    /// Highest price first.
    pub bids: Vec<BookLevel>,
    /// Lowest price first.
    pub asks: Vec<BookLevel>,
    /// When the venue made it, in Unix milliseconds.
    pub timestamp: f64,
}

impl OrderBook {
    /// A book of `bids` and `asks` given as price and amount, in any
    /// order: each side sorted from its best price, with its totals and
    /// running amounts.
    pub fn of(bids: &[(f64, f64)], asks: &[(f64, f64)], timestamp: f64) -> Self {
        Self {
            bids: side(bids.to_vec(), true),
            asks: side(asks.to_vec(), false),
            timestamp,
        }
    }

    /// Reads Ferris's unified book as the old app did: the payload itself,
    /// or the one under its `data` or `orderBook`; levels as `[price,
    /// amount]` pairs or objects (`price` or `px`; `amount`, `size` or
    /// `sz`), numbers or decimal text. A level without a positive price and
    /// amount is left out; a missing or unreadable time is `now`. `None`
    /// when the payload has neither side, so a frame that is no book never
    /// empties the one kept.
    pub fn from_value(payload: &Value, now: f64) -> Option<Self> {
        let record = payload.as_object()?;
        let record = ["data", "orderBook"]
            .into_iter()
            .find_map(|name| record.get(name).and_then(Value::as_object))
            .unwrap_or(record);
        let levels = |name: &str| record.get(name).and_then(Value::as_array);
        let (bids, asks) = (levels("bids"), levels("asks"));
        if bids.is_none() && asks.is_none() {
            return None;
        }
        let read = |levels: Option<&Vec<Value>>| -> Vec<(f64, f64)> {
            levels
                .map(|levels| levels.iter().filter_map(level).collect())
                .unwrap_or_default()
        };
        let timestamp = number(record.get("timestamp"))
            .filter(|at| *at != 0.0)
            .unwrap_or(now);
        Some(Self {
            bids: side(read(bids), true),
            asks: side(read(asks), false),
            timestamp,
        })
    }

    /// The price halfway between the best bid and the best ask; none while
    /// either side is empty (the old `deriveOrderBookStats`' midpoint).
    pub fn midpoint(&self) -> Option<f64> {
        let (bid, ask) = (self.bids.first()?.price, self.asks.first()?.price);
        let midpoint = (bid + ask) / 2.0;
        (bid.is_finite() && ask.is_finite() && midpoint.is_finite()).then_some(midpoint)
    }
}

/// A finite number from a JSON number or decimal text.
fn number(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
    .filter(|number: &f64| number.is_finite())
}

/// One level's price and amount, both positive; `None` for anything else.
fn level(value: &Value) -> Option<(f64, f64)> {
    let (price, amount) = match value {
        Value::Array(pair) => (number(pair.first()), number(pair.get(1))),
        Value::Object(level) => (
            number(level.get("price").or_else(|| level.get("px"))),
            number(
                level
                    .get("amount")
                    .or_else(|| level.get("size"))
                    .or_else(|| level.get("sz")),
            ),
        ),
        _ => (None, None),
    };
    let (price, amount) = (price?, amount?);
    (price > 0.0 && amount > 0.0).then_some((price, amount))
}

/// One side from its best price outward (`bids`: highest first), each
/// level with its total and the running amount through it.
fn side(mut levels: Vec<(f64, f64)>, bids: bool) -> Vec<BookLevel> {
    if bids {
        levels.sort_by(|a, b| b.0.total_cmp(&a.0));
    } else {
        levels.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    let mut cumulative = 0.0;
    levels
        .into_iter()
        .map(|(price, amount)| {
            cumulative += amount;
            BookLevel {
                price,
                amount,
                total: price * amount,
                cumulative,
            }
        })
        .collect()
}

/// The levels `exchange` streams for a book of `depth` levels a side (the
/// old `resolveOrderBookStreamLevels`): the depth itself, held to the most
/// the venue's public book has ([`Venue::book_level_cap`]), then the
/// smallest depth its topics come in that holds it
/// ([`Venue::book_levels`]).
pub fn stream_levels(exchange: &str, depth: u32) -> u32 {
    let venue = Venue::of(exchange);
    let wanted = venue
        .book_level_cap
        .map_or(depth, |cap| depth.min(cap))
        .max(1);
    venue
        .book_levels
        .iter()
        .copied()
        .find(|levels| *levels >= wanted)
        .or(venue.book_levels.last().copied())
        .unwrap_or(wanted)
}

/// `market`'s `orderbook` topic at `levels`.
fn topic(market: &MarketSelection, levels: u32) -> Result<FerrisTopic, ApiError> {
    let mut topic = market.topic(Channel::OrderBook)?;
    if let FerrisTopic::Market { params, .. } = &mut topic {
        params.insert("levels".to_owned(), json!(levels));
    }
    Ok(topic)
}

/// What tells apart the books two selections stream at `depth` (the old
/// `createOrderBookStreamKey`): their topic's key, levels included. Two
/// selections saved differently that stream one topic are one book. `None`
/// for a selection that has no topic.
pub fn stream_key(market: &MarketSelection, depth: u32) -> Option<String> {
    topic(market, stream_levels(&market.exchange, depth))
        .ok()
        .map(|topic| topic.key())
}

/// A feed's book and status.
struct State {
    /// The newest book; none until a snapshot or a frame brings one.
    book: Option<Rc<OrderBook>>,
    /// A live frame was taken in: a snapshot arriving now is older.
    live: bool,
    /// The server refused the topic, or it could not be made.
    failed: bool,
}

/// A market's feed at one depth, shared by its leases.
type Feed = FeedNode<(MarketKey, u32), Book>;

/// What a feed keeps: its market and levels, its book and status, and what
/// streams into them.
struct Book {
    market: MarketSelection,
    levels: u32,
    state: RefCell<State>,
    subscription: RefCell<Option<super::Subscription>>,
    snapshot: RefCell<Option<TaskHandle>>,
}

impl Book {
    fn new(market: &MarketSelection, levels: u32) -> Self {
        Self {
            market: market.clone(),
            levels,
            state: RefCell::new(State {
                book: None,
                live: false,
                failed: false,
            }),
            subscription: RefCell::new(None),
            snapshot: RefCell::new(None),
        }
    }

    fn label(&self) -> String {
        format!(
            "{} {} ({} levels)",
            self.market.exchange, self.market.request_symbol, self.levels
        )
    }
}

/// Starts a new feed: subscribes to its topic, then asks for the book as
/// it stands.
fn start(ferris: &Ferris, feed: &Rc<Feed>) {
    let book = feed.state();
    let label = book.label();
    match topic(&book.market, book.levels) {
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
            *book.subscription.borrow_mut() = Some(subscription);
        }
        Err(error) => {
            log::error!("Order book: cannot stream {label}: {}", error.message);
            book.state.borrow_mut().failed = true;
        }
    }
    let (ferris, weak) = (ferris.clone(), Rc::downgrade(feed));
    let (market, levels) = (book.market.clone(), book.levels);
    let task = TaskHandle::spawn(async move {
        let Some(payload) = snapshot(&ferris, &market, levels, &label).await else {
            return;
        };
        if let Some(feed) = weak.upgrade() {
            receive(&feed, &payload, false);
        }
    });
    *book.snapshot.borrow_mut() = Some(task);
}

/// Takes in a live frame's book, or the snapshot (`live: false`) while no
/// live frame has come.
fn receive(feed: &Feed, payload: &Value, live: bool) {
    {
        let mut state = feed.state().state.borrow_mut();
        if !live && state.live {
            return;
        }
        let Some(book) = OrderBook::from_value(payload, unix_ms()) else {
            return;
        };
        if live {
            // Only live data says the topic streams; a snapshot does not.
            state.live = true;
            state.failed = false;
        }
        state.book = Some(Rc::new(book));
    }
    feed.notify();
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

/// `market`'s book as it stands, `levels` a side, unless its venue serves
/// no book over REST. A failure is logged and the feed goes on live only,
/// as the old app's did.
async fn snapshot(
    ferris: &Ferris,
    market: &MarketSelection,
    levels: u32,
    label: &str,
) -> Option<Value> {
    if let Ok(capabilities) = ferris.capabilities().await
        && Requirement::snapshot(Operation::FetchOrderBook).availability(
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
        mut params,
    } = market.rest_target().ok()?;
    if !Venue::of(&market.exchange).book_depth_by_limit {
        params.insert("levels".to_owned(), json!(levels));
    }
    let body = json!({
        "exchange": exchange,
        "symbol": symbol,
        "params": params,
        "limit": levels,
    });
    match ferris
        .request::<Value>(Operation::FetchOrderBook, &body)
        .await
    {
        Ok(payload) => Some(payload),
        Err(error) => {
            log::warn!(
                "Order book: no snapshot for {label} ({}: {}); streaming only",
                error.code,
                error.message
            );
            None
        }
    }
}

impl Ferris {
    /// A lease on `market`'s book of `depth` levels a side: its shared
    /// feed (by [`MarketSelection::identity`] and the levels the venue
    /// streams for the depth, [`stream_levels`]), started if no lease holds
    /// it. `on_change` is told each time the feed's book or status changes,
    /// until the lease drops.
    pub fn order_book(
        &self,
        market: &MarketSelection,
        depth: u32,
        on_change: impl Fn() + 'static,
    ) -> OrderBookFeed {
        let levels = stream_levels(&market.exchange, depth);
        let lease = self.feeds.hub::<(MarketKey, u32), Book>().lease(
            (market.identity(), levels),
            || Book::new(market, levels),
            |feed| start(self, feed),
            on_change,
        );
        OrderBookFeed(lease)
    }
}

/// One consumer's lease on a market's book (see [`Ferris::order_book`]).
/// Dropping it stops its `on_change`, and the last lease ends the feed.
#[must_use = "dropping an order book feed releases it"]
pub struct OrderBookFeed(FeedLease<(MarketKey, u32), Book>);

impl OrderBookFeed {
    /// The market, as the first lease selected it.
    pub fn market(&self) -> &MarketSelection {
        &self.0.state().market
    }

    /// The levels a side it streams.
    pub fn levels(&self) -> u32 {
        self.0.state().levels
    }

    /// The newest book; none until a snapshot or a frame brought one.
    pub fn book(&self) -> Option<Rc<OrderBook>> {
        self.0.state().state.borrow().book.clone()
    }

    /// The server refused its topic, or it could not be asked for.
    pub fn failed(&self) -> bool {
        self.0.state().state.borrow().failed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_book_reads_pairs_and_objects_and_orders_its_sides() {
        let payload = json!({
            "bids": [[99.5, 2], ["100", "1.5"], [98, 0], [0, 4], ["x", 1]],
            "asks": [
                { "price": 101.5, "amount": 3 },
                { "px": "101", "sz": "0.5" },
                { "price": 102, "size": 1 },
                { "price": 103 },
            ],
            "timestamp": 1_700_000_000_000_i64,
            "nonce": null,
            "symbol": "BTC/USDT:USDT",
        });
        let book = OrderBook::from_value(&payload, 5.0).expect("it has both sides");
        // Best first, levels without a positive price and amount left out.
        let prices = |levels: &[BookLevel]| levels.iter().map(|l| l.price).collect::<Vec<_>>();
        assert_eq!(prices(&book.bids), [100.0, 99.5]);
        assert_eq!(prices(&book.asks), [101.0, 101.5, 102.0]);
        assert_eq!(book.bids[1].cumulative, 3.5);
        assert_eq!(book.bids[1].total, 199.0);
        assert_eq!(book.asks[2].cumulative, 4.5);
        assert_eq!(book.timestamp, 1_700_000_000_000.0);
        assert_eq!(book.midpoint(), Some(100.5));
        assert_eq!(
            book,
            OrderBook::of(
                &[(99.5, 2.0), (100.0, 1.5)],
                &[(101.5, 3.0), (101.0, 0.5), (102.0, 1.0)],
                1_700_000_000_000.0,
            )
        );

        // The old app's envelopes, and a time it could not read.
        for wrapped in [json!({ "data": payload }), json!({ "orderBook": payload })] {
            assert_eq!(OrderBook::from_value(&wrapped, 5.0).as_ref(), Some(&book));
        }
        let untimed = OrderBook::from_value(&json!({ "bids": [[1, 1]] }), 5.0).unwrap();
        assert_eq!((untimed.timestamp, untimed.asks.len()), (5.0, 0));
        // One side has no midpoint; no side is no book.
        assert_eq!(untimed.midpoint(), None);
        for other in [json!(null), json!([]), json!({ "code": "UPSTREAM_ERROR" })] {
            assert_eq!(OrderBook::from_value(&other, 5.0), None);
        }
    }

    #[test]
    fn a_depth_streams_the_levels_its_venue_serves() {
        // Bybit's topics come in four depths.
        for (depth, levels) in [
            (1, 1),
            (10, 50),
            (50, 50),
            (51, 200),
            (200, 200),
            (201, 1000),
        ] {
            assert_eq!(stream_levels("bybit", depth), levels, "bybit {depth}");
        }
        assert_eq!(stream_levels("Bybit", 5_000), 1000);
        // KuCoin's and Bitfinex's public books stop at 100 levels.
        assert_eq!(stream_levels("kucoin", 200), 100);
        assert_eq!(stream_levels("bitfinex", 20), 20);
        // Elsewhere, the depth itself.
        assert_eq!(stream_levels("hyperliquid", 137), 137);
        assert_eq!(stream_levels("nowhere", 0), 1);
    }

    #[test]
    fn selections_that_stream_one_topic_are_one_book() {
        let market = MarketSelection::bybit_btc_usdt();
        let key = stream_key(&market, 200).expect("it has a topic");
        assert_eq!(
            key,
            r#"orderbook:bybit:BTC/USDT:USDT:{"category":"linear","levels":200}"#
        );
        // A depth that streams the same levels, and a selection saved with
        // more metadata.
        assert_eq!(stream_key(&market, 60).as_ref(), Some(&key));
        let saved_again = MarketSelection {
            market_id: Some("BTCUSDT".into()),
            ..market.clone()
        };
        assert_eq!(stream_key(&saved_again, 200).as_ref(), Some(&key));
        // Other levels, another exchange.
        assert_ne!(stream_key(&market, 201).as_ref(), Some(&key));
        let elsewhere = MarketSelection {
            exchange: "binance".into(),
            ..market
        };
        assert_ne!(stream_key(&elsewhere, 200).as_ref(), Some(&key));
    }
}
