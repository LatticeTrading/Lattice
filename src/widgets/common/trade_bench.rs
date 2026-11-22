//! What the tests of the widgets that trade share (the Trade Panel, the
//! Positions): a stand-in wallet, a stubbed Hyperliquid that keeps a book
//! of what was placed and what is held ([`Exchange`]: its HTTP the stub
//! network, its WebSocket a scripted one the test speaks through), a
//! workspace of one widget over them ([`workspace`]) and the started app
//! with all of it in hand ([`Bench`]).
//!
//! The test and the app's root run on different threads, so the app finds
//! its exchange by an id the test puts in its storage ([`RENDEZVOUS`]).

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use nsh_core::{
    element::Root,
    platform::{Idiom, Os},
    prelude::*,
    storage::Storage,
    time::Clock,
};
use nsh_engine::{EngineConfig, HttpBody, HttpRequest};
use nsh_testing::{Headless, MemoryStorage, StubNetwork, StubReply, StubResponse, StubRoute};
use serde_json::{Value, json};

use super::accounts::AddressGroups;
use crate::{
    data::{
        DataConfig, DataServices, ExtraExchange, MakesExchange,
        ferris::FerrisConfig,
        hyperliquid::HyperliquidConfig,
        net::ScriptedSocket,
        trading::paper::{self, Paper},
        wallet::{Event, Reach, WalletConfig, testing::StandIn},
    },
    layout,
    shell::clock::Clock as BarClock,
    storage::StoredWorkspace,
    widgets::{WidgetKind, WidgetSettings, picker::Picker},
};

/// A Hyperliquid no test reaches: its HTTP is the test's stub network.
pub(crate) const HYPERLIQUID: &str = "https://hyperliquid.test";
/// The account the stand-in wallet shares, whole and in short.
pub(crate) const ACCOUNT: &str = "0x14791697260e4c9a71f18484c9f997b308e59325";
pub(crate) const ACCOUNT_SHORT: &str = "0x1479…9325";
/// The exchange's id, as trading knows it.
const VENUE: &str = "hyperliquid";
/// The stubbed universe, in its order: an asset's place is its index.
pub(crate) const COINS: [&str; 4] = ["BTC", "ETH", "MATIC", "kPEPE"];

/// The storage key a test's app finds its stubbed Hyperliquid by: the test
/// and the app's root run on different threads, and storage is what a test
/// gives its app.
pub(crate) const RENDEZVOUS: &str = "lattice.test.exchange";
/// Where Hyperliquid's integration keeps its agents between runs.
pub(crate) const KEPT: &str = "lattice.trading.hyperliquid.v1";

/// What the stubbed Hyperliquid keeps.
pub(crate) struct Book {
    /// The orders that rest, as the open orders are answered.
    pub(crate) resting: Vec<Value>,
    /// The last order id it gave.
    pub(crate) last: u64,
    /// The agent last approved, which is the one it lists.
    pub(crate) agent: Option<String>,
    /// The account whose topics the app subscribed last.
    pub(crate) followed: Option<String>,
    /// What the account is worth.
    pub(crate) equity: String,
    /// The account's fills so far, oldest first, as a snapshot opens with.
    pub(crate) fills: Vec<Value>,
    /// How the account is set to trade each coin it set: its leverage and
    /// whether its margin is the account's. The rest are as [`leverage_of_coin`]
    /// has them.
    pub(crate) leverage: HashMap<String, (u64, bool)>,
    /// The position the account holds, as its perpetuals account lists
    /// one.
    pub(crate) position: Option<Value>,
    /// What each coin is marked at, as its context says it.
    pub(crate) marks: HashMap<String, String>,
    /// The coins whose contexts the app follows now.
    pub(crate) marked: HashSet<String>,
    /// Each coin's best bid and best offer, for those whose book has them.
    pub(crate) quotes: HashMap<String, (String, String)>,
    /// The coins whose best bid and offer the app follows now.
    pub(crate) quoted: HashSet<String>,
    /// Whether no price is said: a subscription to a context or a best bid
    /// and offer is acknowledged, and nothing follows.
    pub(crate) hushed: bool,
}

/// How `coin` is traded until the account sets it: ten times for ETH, five
/// for the rest, on the account's margin.
pub(crate) fn leverage_of_coin(book: &Book, coin: &str) -> (u64, bool) {
    let unset = if coin == "ETH" { 10 } else { 5 };
    book.leverage.get(coin).copied().unwrap_or((unset, true))
}

/// The leverage setting of `coin`, as the account's setting and a
/// position's own name it.
pub(crate) fn leverage_state(book: &Book, coin: &str) -> Value {
    let (value, cross) = leverage_of_coin(book, coin);
    json!({ "type": if cross { "cross" } else { "isolated" }, "value": value })
}

/// The stubbed Hyperliquid: what it keeps, and its WebSocket. Cloning is
/// cheap: every clone is the same exchange.
#[derive(Clone)]
pub(crate) struct Exchange {
    pub(crate) id: String,
    pub(crate) book: Arc<Mutex<Book>>,
    pub(crate) socket: ScriptedSocket,
}

/// The exchanges of the tests running, by id.
static EXCHANGES: Mutex<Vec<Exchange>> = Mutex::new(Vec::new());
static NEXT_EXCHANGE: AtomicU64 = AtomicU64::new(1);

/// The perpetuals account, as both the request and the topic give it.
pub(crate) fn account_state(book: &Book) -> Value {
    json!({
        "assetPositions": book.position.iter().collect::<Vec<_>>(),
        "crossMaintenanceMarginUsed": "7.5",
        "crossMarginSummary": { "accountValue": book.equity },
        "marginSummary": {
            "accountValue": book.equity, "totalMarginUsed": "75.0", "totalNtlPos": "1500.0",
        },
        "time": 1_700_000_000_000_u64,
        "withdrawable": "1159.5",
    })
}

/// The open orders' frame for user.
pub(crate) fn orders_frame(book: &Book, user: &str) -> String {
    json!({ "channel": "openOrders", "data": { "dex": "", "user": user, "orders": book.resting } })
        .to_string()
}

/// The perpetuals account's frame for user.
pub(crate) fn account_frame(book: &Book, user: &str) -> String {
    json!({ "channel": "clearinghouseState", "data": {
        "dex": "", "user": user, "clearinghouseState": account_state(book),
    } })
    .to_string()
}

/// A frame of fills for user: the snapshot a subscription opens with, or
/// fills as they happen.
pub(crate) fn fills_frame(user: &str, fills: &[Value], snapshot: bool) -> String {
    let mut data = json!({ "user": user, "fills": fills });
    if snapshot {
        data["isSnapshot"] = json!(true);
    }
    json!({ "channel": "userFills", "data": data }).to_string()
}

/// A fill of size of coin at price on side (B, A), numbered tid.
pub(crate) fn fill_of(coin: &str, side: &str, price: &str, size: &Value, tid: u64) -> Value {
    json!({
        "coin": coin, "px": price, "sz": size, "side": side,
        "time": 1_700_000_100_000_u64 + tid, "dir": "Open Long", "closedPnl": "0.0",
        "hash": "0x00", "oid": tid, "crossed": true, "fee": "0.01", "tid": tid,
        "feeToken": "USDC",
    })
}

/// The frame of `coin`'s context, which says it is marked at `mark`.
pub(crate) fn mark_frame(coin: &str, mark: &str) -> String {
    json!({ "channel": "activeAssetCtx", "data": { "coin": coin, "ctx": {
        "funding": "0.0000125", "oraclePx": mark, "markPx": mark, "midPx": mark,
    } } })
    .to_string()
}

/// The frame of `coin`'s best bid and best offer.
pub(crate) fn quote_frame(coin: &str, bid: &str, offer: &str) -> String {
    json!({ "channel": "bbo", "data": { "coin": coin, "time": 1_700_000_000_000_u64, "bbo": [
        { "px": bid, "sz": "1.0", "n": 1 }, { "px": offer, "sz": "1.0", "n": 1 },
    ] } })
    .to_string()
}

/// What the stubbed socket answers a frame the app sent with: a pong, an
/// acknowledgement, and after a subscribe's the first frame of its topic.
/// A context is that of a coin it marks, and refused for any other. A best
/// bid and offer are those of a coin whose book has them, acknowledged
/// alone for one it marks with no book (a delisted one), and not answered
/// at all for a coin it does not know: the real Hyperliquid closes the
/// connection over that, so the app must never ask (a test finds what was
/// asked in [`Exchange::subscribed`]).
pub(crate) fn answer(book: &Mutex<Book>, frame: &str) -> Vec<String> {
    let frame: Value = serde_json::from_str(frame).unwrap_or_default();
    let method = frame["method"].as_str().unwrap_or_default();
    if method == "ping" {
        return vec![r#"{"channel":"pong"}"#.to_owned()];
    }
    let subscription = &frame["subscription"];
    let acknowledged = json!({ "channel": "subscriptionResponse", "data": {
        "method": method, "subscription": subscription,
    } })
    .to_string();
    let mut book = book.lock().unwrap();
    if subscription["type"] == "bbo" {
        let coin = subscription["coin"].as_str().unwrap_or_default();
        if method != "subscribe" {
            book.quoted.remove(coin);
            return vec![acknowledged];
        }
        if !book.marks.contains_key(coin) {
            return Vec::new();
        }
        book.quoted.insert(coin.to_owned());
        return match book.quotes.get(coin) {
            Some((bid, offer)) if !book.hushed => {
                vec![acknowledged, quote_frame(coin, bid, offer)]
            }
            _ => vec![acknowledged],
        };
    }
    if subscription["type"] == "activeAssetCtx" {
        let coin = subscription["coin"].as_str().unwrap_or_default();
        if method != "subscribe" {
            book.marked.remove(coin);
            return vec![acknowledged];
        }
        let Some(mark) = book.marks.get(coin).cloned() else {
            let refused = format!("Invalid subscription {subscription}");
            return vec![json!({ "channel": "error", "data": refused }).to_string()];
        };
        book.marked.insert(coin.to_owned());
        return if book.hushed {
            vec![acknowledged]
        } else {
            vec![acknowledged, mark_frame(coin, &mark)]
        };
    }
    if method != "subscribe" {
        return vec![acknowledged];
    }
    let user = subscription["user"].as_str().unwrap_or_default();
    book.followed = Some(user.to_owned());
    let first = match subscription["type"].as_str() {
        Some("openOrders") => orders_frame(&book, user),
        Some("clearinghouseState") => account_frame(&book, user),
        Some("userFills") => fills_frame(user, &book.fills, true),
        _ => return vec![acknowledged],
    };
    vec![acknowledged, first]
}

impl Exchange {
    /// A stubbed Hyperliquid with an account worth $1,234.50 and nothing
    /// else, which a test's app finds by its id. BTC is marked at 61,000,
    /// ETH at 1,891.4, MATIC at 0.37621 and kPEPE at 0.011111; each but
    /// MATIC (delisted, with no book) has a best bid and offer whose middle
    /// is its mark.
    pub(crate) fn new() -> Self {
        let marks = [
            ("BTC", "61000.0"),
            ("ETH", "1891.4"),
            ("MATIC", "0.37621"),
            ("kPEPE", "0.011111"),
        ];
        let book = Arc::new(Mutex::new(Book {
            resting: Vec::new(),
            last: 0,
            agent: None,
            followed: None,
            equity: "1234.5".to_owned(),
            fills: Vec::new(),
            leverage: HashMap::new(),
            position: None,
            marks: marks
                .into_iter()
                .map(|(coin, mark)| (coin.to_owned(), mark.to_owned()))
                .collect(),
            marked: HashSet::new(),
            quotes: [
                ("BTC", "60999.5", "61000.5"),
                ("ETH", "1891.3", "1891.5"),
                ("kPEPE", "0.011110", "0.011112"),
            ]
            .into_iter()
            .map(|(coin, bid, offer)| (coin.to_owned(), (bid.to_owned(), offer.to_owned())))
            .collect(),
            quoted: HashSet::new(),
            hushed: false,
        }));
        let script = book.clone();
        let exchange = Self {
            id: NEXT_EXCHANGE.fetch_add(1, Ordering::Relaxed).to_string(),
            book,
            socket: ScriptedSocket::new(move |frame| answer(&script, frame)),
        };
        EXCHANGES.lock().unwrap().push(exchange.clone());
        exchange
    }

    /// The exchange the test gave the app whose storage this is.
    pub(crate) fn of_this_app() -> Self {
        let id = Storage::get_item(RENDEZVOUS).expect("the test names its exchange");
        let exchanges = EXCHANGES.lock().unwrap();
        let found = exchanges.iter().find(|exchange| exchange.id == id);
        found.expect("the test's exchange is listed").clone()
    }

    /// Sends frame to the account the app follows, if it follows one.
    pub(crate) fn tell(&self, book: &Book, frame: impl Fn(&Book, &str) -> String) {
        if let Some(user) = &book.followed {
            self.socket.push(frame(book, user));
        }
    }

    /// Says how the account's open orders stand now.
    pub(crate) fn push_orders(&self) {
        self.tell(&self.book.lock().unwrap(), orders_frame);
    }

    /// Says how the perpetuals account stands now.
    pub(crate) fn push_account(&self) {
        self.tell(&self.book.lock().unwrap(), account_frame);
    }

    /// The account becomes worth equity, and its topic says so.
    pub(crate) fn worth(&self, equity: &str) {
        let mut book = self.book.lock().unwrap();
        equity.clone_into(&mut book.equity);
        self.tell(&book, account_frame);
    }

    /// An order comes to rest that this app did not place: coin, side (B,
    /// A), price and size, numbered id.
    pub(crate) fn rest(&self, id: u64, coin: &str, side: &str, price: &str, size: &str) {
        let mut book = self.book.lock().unwrap();
        book.resting.push(json!({
            "coin": coin, "side": side, "limitPx": price, "sz": size, "origSz": size,
            "oid": id, "timestamp": 1_700_000_000_000_u64 + id, "reduceOnly": false,
            "orderType": "Limit", "isTrigger": false, "triggerPx": "0.0",
        }));
        self.tell(&book, orders_frame);
    }

    /// The resting order id leaves the book without trading (cancelled
    /// somewhere else).
    pub(crate) fn withdraw(&self, id: u64) {
        let mut book = self.book.lock().unwrap();
        book.resting.retain(|order| order["oid"] != id);
        self.tell(&book, orders_frame);
    }

    /// The market fills the resting order id, whole, at its price.
    pub(crate) fn fill(&self, id: u64) {
        let mut book = self.book.lock().unwrap();
        let Some(at) = book.resting.iter().position(|order| order["oid"] == id) else {
            panic!("no order {id} rests");
        };
        let order = book.resting.remove(at);
        let fill = fill_of(
            order["coin"].as_str().unwrap(),
            order["side"].as_str().unwrap(),
            order["limitPx"].as_str().unwrap(),
            &order["sz"],
            id,
        );
        book.fills.push(fill.clone());
        self.tell(&book, orders_frame);
        self.tell(&book, |_, user| {
            fills_frame(user, std::slice::from_ref(&fill), false)
        });
    }

    /// A trade of the account's that no order here made: size of coin
    /// sold at price.
    pub(crate) fn sold(&self, tid: u64, coin: &str, price: &str, size: &str) {
        let mut book = self.book.lock().unwrap();
        let fill = fill_of(coin, "A", price, &json!(size), tid);
        book.fills.push(fill.clone());
        self.tell(&book, |_, user| {
            fills_frame(user, std::slice::from_ref(&fill), false)
        });
    }

    /// The account comes to hold `size` of `coin` (below zero: short),
    /// entered at `entry` and worth `value` now, and its topic says so.
    pub(crate) fn hold(&self, coin: &str, size: &str, entry: &str, value: &str, pnl: &str) {
        let mut book = self.book.lock().unwrap();
        book.position = Some(json!({ "type": "oneWay", "position": {
            "coin": coin, "szi": size, "leverage": leverage_state(&book, coin),
            "entryPx": entry, "positionValue": value, "unrealizedPnl": pnl,
            "returnOnEquity": "0.0833", "liquidationPx": null, "marginUsed": "122.0",
            "cumFunding": { "sinceOpen": "0.0" },
        } }));
        self.tell(&book, account_frame);
    }

    /// `coin` comes to be marked at `price`, and its context says so to an
    /// app that follows it.
    pub(crate) fn mark(&self, coin: &str, price: &str) {
        let mut book = self.book.lock().unwrap();
        book.marks.insert(coin.to_owned(), price.to_owned());
        if book.marked.contains(coin) && !book.hushed {
            self.socket.push(mark_frame(coin, price));
        }
    }

    /// `coin`'s best bid and best offer come to be `bid` and `offer`, and
    /// an app that follows them is told.
    pub(crate) fn quote(&self, coin: &str, bid: &str, offer: &str) {
        let mut book = self.book.lock().unwrap();
        let quote = (bid.to_owned(), offer.to_owned());
        book.quotes.insert(coin.to_owned(), quote);
        if book.quoted.contains(coin) && !book.hushed {
            self.socket.push(quote_frame(coin, bid, offer));
        }
    }

    /// No price is said any more (`hushed`), or prices are said again: a
    /// subscription to a context or a best bid and offer is acknowledged,
    /// and nothing follows.
    pub(crate) fn hush(&self, hushed: bool) {
        self.book.lock().unwrap().hushed = hushed;
    }

    /// An exit of the whole position in `coin` comes to wait that this app
    /// did not place: a take profit or a stop loss which sells (`A`) or
    /// buys (`B`) once `trigger` is reached, numbered `id`.
    pub(crate) fn exit(&self, id: u64, coin: &str, side: &str, trigger: &str, take_profit: bool) {
        let mut book = self.book.lock().unwrap();
        let kind = if take_profit {
            "Take Profit Market"
        } else {
            "Stop Market"
        };
        book.resting.push(json!({
            "coin": coin, "side": side, "limitPx": trigger, "sz": "0.0", "origSz": "0.0",
            "oid": id, "timestamp": 1_700_000_000_000_u64 + id, "reduceOnly": true,
            "orderType": kind, "isTrigger": true, "triggerPx": trigger,
            "isPositionTpsl": true, "children": [],
        }));
        self.tell(&book, orders_frame);
    }

    /// The account closes what it held, and its topic says so.
    pub(crate) fn flat(&self) {
        let mut book = self.book.lock().unwrap();
        book.position = None;
        self.tell(&book, account_frame);
    }

    /// What the app subscribed over the socket so far, in order.
    pub(crate) fn subscribed(&self) -> Vec<Value> {
        let frames = self.socket.sent().into_iter();
        frames
            .filter_map(|frame| serde_json::from_str::<Value>(&frame).ok())
            .filter(|frame| frame["method"] == "subscribe")
            .map(|frame| frame["subscription"].clone())
            .collect()
    }
}

/// A request's JSON body.
pub(crate) fn body_of(request: &HttpRequest) -> Value {
    match &request.body {
        HttpBody::Bytes(bytes) => serde_json::from_slice(bytes).unwrap_or_default(),
        _ => Value::Null,
    }
}

/// Answers the stubbed Hyperliquid's info requests: four perpetuals (one
/// delisted), their mid prices, the account's perpetuals account, the
/// orders that rest in its book and the agent it lists.
pub(crate) fn stub_info(exchange: &Exchange, request: &HttpRequest, reply: StubReply) {
    let body = body_of(request);
    let mut response = StubResponse::default();
    let book = exchange.book.lock().unwrap();
    let answer = match body["type"].as_str().unwrap_or_default() {
        "meta" => json!({ "universe": [
            { "szDecimals": 5, "name": "BTC", "maxLeverage": 40 },
            { "szDecimals": 4, "name": "ETH", "maxLeverage": 25 },
            { "szDecimals": 1, "name": "MATIC", "maxLeverage": 20, "isDelisted": true },
            { "szDecimals": 0, "name": "kPEPE", "maxLeverage": 10 },
        ] }),
        "allMids" => json!({ "BTC": "61012.7", "ETH": "1891.4", "kPEPE": "0.011111" }),
        "clearinghouseState" => account_state(&book),
        "frontendOpenOrders" => Value::Array(book.resting.clone()),
        "extraAgents" => book
            .agent
            .iter()
            .map(|agent| {
                json!({
                    "name": "Lattice", "address": agent, "validUntil": 4_102_444_800_000_u64,
                })
            })
            .collect(),
        "activeAssetData" => match body["coin"].as_str() {
            Some(coin) if COINS.contains(&coin) => json!({
                "user": body["user"], "coin": coin, "leverage": leverage_state(&book, coin),
                "maxTradeSzs": ["1.0", "1.0"], "availableToTrade": ["1000.0", "1000.0"],
                "markPx": "61000.0",
            }),
            _ => Value::Null,
        },
        _ => {
            response.status = 422;
            response.body = b"Failed to deserialize the JSON body into the target type".to_vec();
            reply.send(response);
            return;
        }
    };
    response.status = 200;
    response.body = serde_json::to_vec(&answer).unwrap();
    reply.send(response);
}

/// An exit as the open orders list one: the trigger order wire of an
/// action, numbered id.
pub(crate) fn exit_listed(wire: &Value, id: u64, of_position: bool) -> Value {
    let trigger = &wire["t"]["trigger"];
    let kind = if trigger["tpsl"] == "tp" {
        "Take Profit Market"
    } else {
        "Stop Market"
    };
    let size = if of_position {
        json!("0.0")
    } else {
        wire["s"].clone()
    };
    json!({
        "coin": COINS[wire["a"].as_u64().unwrap() as usize],
        "side": if wire["b"] == true { "B" } else { "A" },
        "limitPx": wire["p"], "sz": size, "origSz": size,
        "oid": id, "timestamp": 1_700_000_000_000_u64 + id,
        "reduceOnly": wire["r"], "orderType": kind, "isTrigger": true,
        "triggerPx": trigger["triggerPx"], "isPositionTpsl": of_position, "children": [],
    })
}

/// What is left of the position `held` (as the perpetuals account lists
/// one) once `size` of it is closed; nothing when that is all of it.
fn reduced(held: &Value, size: f64) -> Option<Value> {
    let figure = |name: &str| {
        let text = held["position"][name].as_str();
        text.and_then(|text| text.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    let (signed, value) = (figure("szi"), figure("positionValue"));
    // To the hundred-millionth: what a float leaves of 0.01 less 0.005.
    let left = ((signed.abs() - size).max(0.0) * 1e8).round() / 1e8;
    if left == 0.0 {
        return None;
    }
    let mut held = held.clone();
    held["position"]["szi"] = json!(format!("{}", left.copysign(signed)));
    held["position"]["positionValue"] = json!(format!("{}", value * left / signed.abs()));
    Some(held)
}

/// Answers the stubbed Hyperliquid's exchange endpoint: an agent is
/// approved; a leverage is set (the margin mode of a coin held is not
/// changed, which it refuses); an order worth under $10 is refused, one
/// that cancels what does not fill is filled at 61,000 (one that only
/// reduces for no more than is held of its coin the other way, which it
/// takes off the position, and refused with nothing to reduce), any other
/// rests in the book, with the exits sent with it as its children until it
/// trades; a position's exits rest as orders of no size; a cancel takes
/// its order out of the book. What changes is said over the socket too,
/// as Hyperliquid says it.
pub(crate) fn stub_exchange(exchange: &Exchange, request: &HttpRequest, reply: StubReply) {
    let action = &body_of(request)["action"];
    let mut book = exchange.book.lock().unwrap();
    let figure = |value: &Value| value.as_str().and_then(|text| text.parse::<f64>().ok());
    let refuse = |why: &str, reply: StubReply| {
        let mut response = StubResponse::default();
        response.status = 200;
        response.body = serde_json::to_vec(&json!({ "status": "err", "response": why })).unwrap();
        reply.send(response);
    };
    let made = match action["type"].as_str().unwrap_or_default() {
        "approveAgent" => {
            book.agent = action["agentAddress"].as_str().map(str::to_owned);
            json!({ "type": "default" })
        }
        "updateLeverage" => {
            let coin = COINS[action["asset"].as_u64().unwrap() as usize];
            let cross = action["isCross"] == true;
            let held = book
                .position
                .as_ref()
                .is_some_and(|held| held["position"]["coin"] == coin);
            if held && leverage_of_coin(&book, coin).1 != cross {
                return refuse("Cannot switch leverage type with open position.", reply);
            }
            let times = action["leverage"].as_u64().unwrap();
            book.leverage.insert(coin.to_owned(), (times, cross));
            json!({ "type": "default" })
        }
        "order" if action["grouping"] == "positionTpsl" => {
            let exits = action["orders"].as_array().unwrap();
            let mut statuses = Vec::new();
            for exit in exits {
                book.last += 1;
                let id = 1000 + book.last;
                book.resting.push(exit_listed(exit, id, true));
                statuses.push(json!("waitingForTrigger"));
            }
            exchange.tell(&book, orders_frame);
            json!({ "type": "order", "data": { "statuses": statuses } })
        }
        "order" => {
            let orders = action["orders"].as_array().unwrap();
            let (order, exits) = (&orders[0], &orders[1..]);
            let worth = figure(&order["p"]).unwrap_or(0.0) * figure(&order["s"]).unwrap_or(0.0);
            let coin = COINS[order["a"].as_u64().unwrap() as usize];
            let side = if order["b"] == true { "B" } else { "A" };
            let immediate = order["t"]["limit"]["tif"] == "Ioc";
            // What it may close: what is held of the coin, the other way.
            let closable = book.position.as_ref().and_then(|held| {
                let held = &held["position"];
                let signed = figure(&held["szi"]).filter(|_| held["coin"] == coin)?;
                ((signed > 0.0) != (side == "B")).then_some(signed.abs())
            });
            let reduces = immediate && order["r"] == true;
            let mut statuses = Vec::new();
            if worth < 10.0 {
                statuses.push(json!({ "error": "Order must have minimum value of $10." }));
            } else if reduces && closable.is_none() {
                statuses.push(json!({ "error": "Reduce only order would increase position." }));
            } else {
                book.last += 1;
                let id = 1000 + book.last;
                let children: Vec<Value> = exits
                    .iter()
                    .map(|exit| {
                        book.last += 1;
                        exit_listed(exit, 1000 + book.last, false)
                    })
                    .collect();
                if immediate {
                    // One that only reduces trades no more than is held,
                    // and leaves the rest of the position, or none.
                    let asked = figure(&order["s"]).unwrap_or(0.0);
                    let traded = match closable.filter(|_| reduces) {
                        Some(held) if held < asked => json!(format!("{held}")),
                        _ => order["s"].clone(),
                    };
                    if reduces {
                        let closed = figure(&traded).unwrap_or(0.0);
                        let left = book
                            .position
                            .as_ref()
                            .and_then(|held| reduced(held, closed));
                        book.position = left;
                        exchange.tell(&book, account_frame);
                    }
                    let fill = fill_of(coin, side, "61000.0", &traded, id);
                    book.fills.push(fill.clone());
                    exchange.tell(&book, |_, user| {
                        fills_frame(user, std::slice::from_ref(&fill), false)
                    });
                    statuses.push(json!({
                        "filled": { "totalSz": traded, "avgPx": "61000.0", "oid": id }
                    }));
                    // It traded: its exits wait for their triggers now.
                    statuses.extend(children.iter().map(|_| json!("waitingForTrigger")));
                    book.resting.extend(children);
                } else {
                    statuses.push(json!({ "resting": { "oid": id } }));
                    statuses.extend(children.iter().map(|_| json!("waitingForFill")));
                    book.resting.push(json!({
                        "coin": coin, "side": side,
                        "limitPx": order["p"], "sz": order["s"], "origSz": order["s"],
                        "oid": id, "timestamp": 1_700_000_000_000_u64 + id,
                        "reduceOnly": order["r"], "orderType": "Limit",
                        "isTrigger": false, "triggerPx": "0.0", "children": children,
                    }));
                }
                exchange.tell(&book, orders_frame);
            }
            json!({ "type": "order", "data": { "statuses": statuses } })
        }
        "cancel" => {
            let id = &action["cancels"][0]["o"];
            let before = book.resting.len();
            book.resting.retain(|order| order["oid"] != *id);
            let status = if book.resting.len() < before {
                exchange.tell(&book, orders_frame);
                json!("success")
            } else {
                json!({ "error": "Order was never placed, already canceled, or filled." })
            };
            json!({ "type": "cancel", "data": { "statuses": [status] } })
        }
        _ => return refuse("Unknown action", reply),
    };
    let mut response = StubResponse::default();
    response.status = 200;
    response.body = serde_json::to_vec(&json!({ "status": "ok", "response": made })).unwrap();
    reply.send(response);
}

/// A workspace of one `kind` widget with `saved` as its settings (none:
/// its defaults), over data services whose wallet is a stand-in and whose
/// Hyperliquid is the test's stubbed one (its HTTP the stub network, its
/// WebSocket the exchange's scripted one). Under it, what a test does that
/// not every widget has a control for: the wallet connects, its account
/// enables trading on Hyperliquid (where that stands is said beside it),
/// and the wallet changes its account.
pub(crate) fn workspace(kind: WidgetKind, saved: Option<Value>) -> impl IntoElement {
    workspace_over(kind, saved, false)
}

/// [`workspace`] over services that also list Paper
/// ([`paper`](crate::data::trading::paper)), an exchange that is none and
/// has nothing a venue may leave out: what shows that a widget knows no
/// exchange. The account enables trading there with a control of its own.
pub(crate) fn workspace_with_paper(kind: WidgetKind, saved: Option<Value>) -> impl IntoElement {
    workspace_over(kind, saved, true)
}

fn workspace_over(kind: WidgetKind, saved: Option<Value>, with_paper: bool) -> impl IntoElement {
    let extra = with_paper.then(|| {
        let make: Rc<MakesExchange> = Rc::new(|status| {
            let paper = Paper::new(status);
            (Rc::new(paper.clone()), Rc::new(paper))
        });
        make
    });
    let wallets = StandIn::new(Reach::Page, &["Rabby Wallet"]);
    assert_eq!(format!("{:#x}", wallets.address()), ACCOUNT);
    let services = DataServices::provide_with(DataConfig {
        ferris: FerrisConfig {
            http_base: "https://ferris.test".to_owned(),
            ws_url: "ws://127.0.0.1:9/v1/ws".to_owned(),
            ..FerrisConfig::production()
        },
        hyperliquid: HyperliquidConfig {
            http_base: HYPERLIQUID.to_owned(),
            transport: Some(Exchange::of_this_app().socket.transport()),
            ..HyperliquidConfig::production()
        },
        wallet: WalletConfig {
            connector: Some(Rc::new(wallets.clone())),
        },
        extra: ExtraExchange(extra),
        ..DataConfig::default()
    });
    // What the account widgets read of the shell.
    provide_context(BarClock::restore());
    AddressGroups::provide();
    let mut stored = StoredWorkspace::default();
    layout::normalize(&mut stored);
    layout::spawn(&mut stored, kind);
    if let Some(saved) = saved {
        let widget = layout::widget_ids(&stored)
            .into_iter()
            .next()
            .expect("the widget just added");
        let settings = WidgetSettings::read(kind, saved);
        assert!(layout::set_tab_settings(&mut stored, widget, &settings));
    }
    let stored = signal(stored);
    let picker = Picker::new();
    services
        .sessions
        .follow(move || layout::widget_ids(&stored.get()));
    let another = Event::Accounts {
        wallet: "w1".to_owned(),
        accounts: json!(["0x5e9ee1089755c3435139848e47e6635505d5a13a"]),
    };
    let does = |label: &'static str, act: Rc<dyn Fn()>| {
        view()
            .role(AccessibilityRole::Button)
            .label(label)
            .on_press(move || act())
            .child(text(label))
    };
    let (wallet, trading, stands) = (
        services.wallet.clone(),
        services.trading.clone(),
        services.trading.clone(),
    );
    let (on_paper, paper_stands) = (services.trading.clone(), services.trading.clone());
    view()
        .size_full()
        .flex_col()
        .child(layout::workspace(stored, picker))
        .child(picker.view(stored))
        .child(
            view()
                .flex_row()
                .gap(px(12.0))
                .child(does(
                    "the wallet connects",
                    Rc::new(move || wallet.connect()),
                ))
                .child(does(
                    "the account enables trading",
                    Rc::new(move || trading.enable(VENUE)),
                ))
                .child(does(
                    "the account enables trading on Paper",
                    Rc::new(move || on_paper.enable(paper::VENUE)),
                ))
                .child(does(
                    "the wallet changes account",
                    Rc::new(move || wallets.say(another.clone())),
                ))
                .child(text(move || {
                    format!("hyperliquid is {:?}", stands.access(VENUE))
                }))
                .child(text(move || {
                    format!("paper is {:?}", paper_stands.access(paper::VENUE))
                })),
        )
}

/// A started app, with what a test reads of it and does to it.
pub(crate) struct Bench {
    pub(crate) app: Headless,
    /// The stub network, which keeps the requests made.
    pub(crate) network: Arc<StubNetwork>,
    /// The stubbed Hyperliquid, which the test has act as a market would.
    pub(crate) exchange: Exchange,
    /// What the app saved.
    pub(crate) storage: Arc<MemoryStorage>,
}

/// The app rooted at root over the stubbed Hyperliquid, and the stub
/// network, which keeps the requests made.
pub(crate) fn start(root: Root) -> (Headless, Arc<StubNetwork>) {
    start_in(root, 1280.0)
}

/// [`start`] in a window width wide.
pub(crate) fn start_in(root: Root, width: f32) -> (Headless, Arc<StubNetwork>) {
    let bench = start_on(
        root,
        width,
        &Exchange::new(),
        BTreeMap::new(),
        Clock::system(),
    );
    (bench.app, bench.network)
}

/// [`start`], with the exchange and the storage in hand.
pub(crate) fn start_live(root: Root) -> Bench {
    start_on(
        root,
        1280.0,
        &Exchange::new(),
        BTreeMap::new(),
        Clock::system(),
    )
}

/// The app rooted at root in a window width wide, over exchange and
/// storage that holds saved, on clock.
pub(crate) fn start_on(
    root: Root,
    width: f32,
    exchange: &Exchange,
    mut saved: BTreeMap<String, String>,
    clock: Clock,
) -> Bench {
    let (info, acts) = (exchange.clone(), exchange.clone());
    // Delayed replies pass on the app's clock, so a test that moves it
    // moves them.
    let network = Arc::new(
        StubNetwork::new([
            (
                format!("POST {HYPERLIQUID}/info"),
                StubRoute::Handler(Arc::new(move |request: HttpRequest, reply: StubReply| {
                    stub_info(&info, &request, reply);
                })),
            ),
            (
                format!("POST {HYPERLIQUID}/exchange"),
                StubRoute::Handler(Arc::new(move |request: HttpRequest, reply: StubReply| {
                    stub_exchange(&acts, &request, reply);
                })),
            ),
        ])
        .with_clock(clock.clone()),
    );
    saved.insert(RENDEZVOUS.to_owned(), exchange.id.clone());
    let storage = MemoryStorage::new(saved);
    let mut config = EngineConfig::new(root);
    config.viewport = Size::new(px(width), px(800.0));
    config.storage = Some(storage.clone());
    config.network = Some(network.clone());
    config.clock = clock;
    config.environment.os = Os::Windows;
    config.environment.idiom = Idiom::Desktop;
    let app = Headless::start(config).expect("the app starts");
    Bench {
        app,
        network,
        exchange: exchange.clone(),
        storage,
    }
}

/// The bodies of the requests made of the stubbed Hyperliquid's `endpoint`
/// (`info`, `exchange`) so far, in order.
pub(crate) fn sent(network: &StubNetwork, endpoint: &str) -> Vec<Value> {
    let url = format!("{HYPERLIQUID}/{endpoint}");
    network
        .requests()
        .iter()
        .filter(|request| request.url == url)
        .map(body_of)
        .collect()
}

/// The actions posted to the exchange endpoint so far, but for the
/// agent's approval.
pub(crate) fn acted(network: &StubNetwork) -> Vec<Value> {
    sent(network, "exchange")
        .into_iter()
        .map(|posted| posted["action"].clone())
        .filter(|action| action["type"] != "approveAgent")
        .collect()
}
