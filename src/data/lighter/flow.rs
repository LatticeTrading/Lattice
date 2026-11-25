//! Trading on Lighter from end to end: the app's own trading
//! ([`Trading`](crate::data::Trading), as the widgets ask it) over a
//! stand-in wallet and a stubbed Lighter ([`Stub`]) that answers as the live
//! one did on 2026-11-22 and takes transactions as it would: by their
//! nonce, from a key it lists.
//!
//! What it shows is the way from a wallet through a registered key to
//! orders, and what is posted on it. What is posted is then checked here as
//! Lighter checks it: each transaction's signature is its key's of its hash,
//! and the key's registration is signed by the account's wallet.
//!
//! The test and the app's root run on different threads, so the app finds
//! its stub by an id the test puts in its storage ([`RENDEZVOUS`]).

use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use alloy::primitives::{Address, Signature, hex};
use nsh_core::{
    element::Root,
    platform::{Idiom, Os},
    prelude::*,
    storage::Storage,
};
use nsh_engine::{EngineConfig, HttpBody, HttpRequest};
use nsh_testing::{Headless, MemoryStorage, StubNetwork, StubReply, StubResponse, StubRoute, WAIT};
use serde_json::{Value, json};

use super::{
    LighterConfig,
    crypto::{SecretKey, verifies},
    tx::{Envelope, Grouping, Network, Order, OrderType, Standing, Transaction, register_message},
};
use crate::{
    data::{
        DataConfig, DataServices,
        ferris::FerrisConfig,
        hyperliquid::HyperliquidConfig,
        net::{QueryState, ScriptedSocket, Watch},
        portfolio::{Leverage, MarginAccount, MarginMode},
        trading::{
            AccountEvent, CancelRequest, Doing, Exits, Following, OpenOrders, OrderKind,
            OrderRequest, Side, TimeInForce, TradingVenue,
        },
        wallet::{Reach, WalletConfig, testing::StandIn},
    },
    layout,
    shell::clock::Clock as BarClock,
    storage::StoredWorkspace,
    widgets::{WidgetKind, WidgetSettings, common::accounts::AddressGroups, picker::Picker},
};

/// A Lighter no test reaches: its HTTP is the test's stub network.
const LIGHTER: &str = "https://lighter.test";
/// The exchange's id, as trading knows it.
const VENUE: &str = "lighterxyz";
/// The account the stand-in wallet shares, and its main account's index on
/// the stubbed Lighter.
const ACCOUNT: &str = "0x14791697260e4c9a71f18484c9f997b308e59325";
const INDEX: u64 = 2_180;
/// Where Lighter's integration keeps its keys between runs.
const KEPT: &str = "lattice.trading.lighterxyz.v1";
/// The storage key a test's app finds its stub by.
const RENDEZVOUS: &str = "lattice.test.lighter";
/// The index the app registers its key at on an account whose other keys
/// are Lighter's own, a bot's and other software's default.
const KEY_INDEX: u8 = 253;

/// What the stubbed Lighter keeps.
#[derive(Default)]
struct Books {
    /// The public keys it lists, by index.
    keys: BTreeMap<u8, String>,
    /// The nonce each key's next transaction takes.
    nonces: BTreeMap<u8, u64>,
    /// The transactions it took, in order: each kind's number and its JSON.
    taken: Vec<(u8, Value)>,
    /// The auth tokens the open orders were asked for with.
    tokens: Vec<String>,
    /// The orders that rest or wait, as the open orders are answered.
    orders: Vec<Value>,
    /// The last order index it gave.
    last: u64,
    /// How the account is set to trade BTC: its margin fraction as its
    /// entry writes it, and whether its margin is the position's own.
    setting: Option<(String, bool)>,
    /// What the next transaction is refused with, once.
    refusing: Option<&'static str>,
}

/// The stubbed Lighter, and its socket. Cloning is cheap: every clone is
/// the same one.
#[derive(Clone)]
struct Stub {
    id: String,
    books: Arc<Mutex<Books>>,
    socket: ScriptedSocket,
}

/// Every test's stub, for its app to find.
static STUBS: Mutex<Vec<Stub>> = Mutex::new(Vec::new());
static NEXT_STUB: AtomicU64 = AtomicU64::new(1);

impl Stub {
    /// A stubbed Lighter that lists three keys of other people's for the
    /// account: Lighter's own site's, a bot's, and other software's default.
    fn new() -> Self {
        let books = Books {
            keys: [
                (0, "aa".repeat(40)),
                (4, "bb".repeat(40)),
                (254, "cc".repeat(40)),
            ]
            .into(),
            nonces: [(0, 7), (4, 2)].into(),
            ..Books::default()
        };
        let stub = Self {
            id: NEXT_STUB.fetch_add(1, Ordering::Relaxed).to_string(),
            books: Arc::new(Mutex::new(books)),
            // A ping's pong, BTC's statistics when they are subscribed, and
            // nothing else: the rest of what shows is what the requests
            // answer.
            socket: ScriptedSocket::new(|frame| {
                if frame.contains(r#""ping""#) {
                    vec![r#"{"type":"pong"}"#.to_owned()]
                } else if frame.contains(r#""subscribe""#) && frame.contains("market_stats/1") {
                    vec![statistics().to_string()]
                } else {
                    Vec::new()
                }
            }),
        };
        STUBS.lock().unwrap().push(stub.clone());
        stub
    }

    /// The stub the test gave the app whose storage this is.
    fn of_this_app() -> Self {
        let id = Storage::get_item(RENDEZVOUS).expect("the test names its stub");
        let stubs = STUBS.lock().unwrap();
        let found = stubs.iter().find(|stub| stub.id == id);
        found.expect("the test's stub is listed").clone()
    }

    fn books(&self) -> std::sync::MutexGuard<'_, Books> {
        self.books.lock().unwrap()
    }

    /// The transactions taken so far of the kind numbered `kind`.
    fn taken(&self, kind: u8) -> Vec<Value> {
        let books = self.books();
        let of_kind = books.taken.iter().filter(|(taken, _)| *taken == kind);
        of_kind.map(|(_, info)| info.clone()).collect()
    }
}

/// BTC's statistics, as a subscription to them opens: marked at 61,000,
/// the middle of its book between two of its prices.
fn statistics() -> Value {
    json!({
        "channel": "market_stats:1",
        "market_stats": {
            "symbol": "BTC", "market_id": 1, "index_price": "61001.0",
            "mark_price": "61000.0", "mid_price": "61012.74",
            "best_ask_price": "61013.3", "best_bid_price": "61012.2",
        },
        "type": "subscribed/market_stats",
    })
}

/// The address's accounts: its main one, long 0.01 BTC.
fn accounts(books: &Books) -> Value {
    let (fraction, isolated) = books
        .setting
        .clone()
        .unwrap_or_else(|| ("5.00".to_owned(), false));
    json!({ "code": 200, "total": 1, "accounts": [{
        "code": 0, "account_type": 0, "index": INDEX, "l1_address": ACCOUNT,
        "available_balance": "1203.500000", "collateral": "1224.500000",
        "total_asset_value": "1234.500000", "cross_asset_value": "1234.500000",
        "cross_initial_margin_requirement": "30.500000",
        "cross_maintenance_margin_requirement": "7.320000",
        "positions": [
            { "market_id": 1, "symbol": "BTC", "initial_margin_fraction": fraction,
              "sign": 1, "position": "0.01000", "avg_entry_price": "60000.0",
              "position_value": "610.000000", "unrealized_pnl": "10.000000",
              "liquidation_price": "0", "total_funding_paid_out": "0.000000",
              "margin_mode": i64::from(isolated), "allocated_margin": "0.000000" },
        ],
        "assets": [{ "symbol": "USDC", "asset_id": 3, "balance": "0.000000",
                     "locked_balance": "0.000000" }],
    }] })
}

/// What it trades: BTC, and a market that is not trading.
fn markets() -> Value {
    json!({ "code": 200, "order_book_details": [
        { "symbol": "BTC", "market_id": 1, "market_type": "perp", "status": "active",
          "min_base_amount": "0.00007", "min_quote_amount": "10.000000",
          "size_decimals": 5, "price_decimals": 1,
          "default_initial_margin_fraction": 500, "min_initial_margin_fraction": 200,
          "mark_price": "61000.0",
          "market_config": { "market_margin_mode": 0, "force_reduce_only": false } },
        { "symbol": "EDEN", "market_id": 89, "status": "inactive",
          "min_base_amount": "100.0", "min_quote_amount": "10.000000",
          "size_decimals": 1, "price_decimals": 5,
          "default_initial_margin_fraction": 2000, "min_initial_margin_fraction": 2000,
          "market_config": { "market_margin_mode": 0, "force_reduce_only": false } },
    ], "spot_order_book_details": [] })
}

/// An order of a transaction, as the open orders list it: numbered
/// `index`, waiting for the order numbered `parent` when it is its exit.
fn listed(order: &Value, index: u64, parent: Option<u64>) -> Value {
    let units = |name: &str, scale: f64, places: usize| {
        let units = order[name].as_u64().unwrap_or_default() as f64;
        format!("{:.places$}", units / scale)
    };
    let kind = match order["Type"].as_u64() {
        Some(1) => "market",
        Some(2) => "stop-loss",
        Some(4) => "take-profit",
        _ => "limit",
    };
    let waits = order["TriggerPrice"].as_u64().unwrap_or_default() > 0;
    json!({
        "order_index": index, "order_id": index.to_string(), "market_index": 1,
        "market_symbol": "BTC", "owner_account_index": INDEX,
        "initial_base_amount": units("BaseAmount", 1e5, 5),
        "remaining_base_amount": units("BaseAmount", 1e5, 5),
        "price": units("Price", 10.0, 1), "is_ask": order["IsAsk"] == 1, "type": kind,
        "reduce_only": order["ReduceOnly"] == 1,
        "trigger_price": units("TriggerPrice", 10.0, 1),
        "status": if waits { "pending" } else { "open" },
        "trigger_status": match (waits, parent) {
            (_, Some(_)) => "parent-order",
            (true, None) => "mark-price",
            (false, None) => "na",
        },
        "parent_order_index": parent.unwrap_or(0),
        "timestamp": 1_795_000_000 + index,
    })
}

/// A reply of `status` with `body`.
fn answer(reply: StubReply, status: u16, body: &Value) {
    let mut response = StubResponse::default();
    response.status = status;
    response.body = serde_json::to_vec(body).expect("the answer is JSON");
    reply.send(response);
}

/// A refusal as Lighter writes one.
fn refuse(reply: StubReply, code: i64, message: &str) {
    answer(reply, 400, &json!({ "code": code, "message": message }));
}

/// Answers what the stubbed Lighter is asked.
fn stub_get(stub: &Stub, request: &HttpRequest, reply: StubReply) {
    let Ok(url) = url::Url::parse(&request.url) else {
        return refuse(reply, 20_001, "invalid param");
    };
    let asked = |name: &str| {
        let found = url.query_pairs().find(|(key, _)| key == name);
        found
            .map(|(_, value)| value.into_owned())
            .unwrap_or_default()
    };
    let mut books = stub.books();
    match url.path() {
        "/api/v1/account" if asked("value") == ACCOUNT => answer(reply, 200, &accounts(&books)),
        "/api/v1/account" => refuse(reply, 21_100, "account not found"),
        "/api/v1/orderBookDetails" => answer(reply, 200, &markets()),
        "/api/v1/orderBookOrders" => answer(
            reply,
            200,
            &json!({ "code": 200, "total_asks": 1, "asks": [{ "price": "61000.5" }],
                     "total_bids": 1, "bids": [{ "price": "60999.5" }] }),
        ),
        "/api/v1/nextNonce" => {
            let index: u8 = asked("api_key_index").parse().unwrap_or_default();
            let nonce = books.nonces.get(&index).copied().unwrap_or(0);
            answer(reply, 200, &json!({ "code": 200, "nonce": nonce }));
        }
        "/api/v1/apikeys" => {
            let index: u8 = asked("api_key_index").parse().unwrap_or_default();
            let entry = |(index, public): (&u8, &String)| {
                json!({ "account_index": INDEX, "api_key_index": index, "nonce": 0,
                        "public_key": public })
            };
            let keys: Vec<Value> = books
                .keys
                .iter()
                .filter(|(listed, _)| index == 255 || **listed == index)
                .map(entry)
                .collect();
            if keys.is_empty() {
                refuse(reply, 21_109, "api key not found");
            } else {
                answer(reply, 200, &json!({ "code": 200, "api_keys": keys }));
            }
        }
        "/api/v1/accountActiveOrders" => {
            let token = request
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                .map(|(_, token)| token.clone());
            let Some(token) = token else {
                let missing = "invalid param : auth query param and Authorization header are empty";
                return refuse(reply, 20_001, missing);
            };
            books.tokens.push(token);
            answer(reply, 200, &json!({ "code": 200, "orders": books.orders }));
        }
        _ => refuse(reply, 20_001, "invalid param"),
    }
}

/// Takes a transaction as Lighter would: of a key it lists (or the key the
/// transaction registers), with that key's next nonce. A key is listed, an
/// order rests or waits, a cancel takes its order out, and a leverage is
/// set.
fn stub_send(stub: &Stub, request: &HttpRequest, reply: StubReply) {
    let HttpBody::Bytes(body) = &request.body else {
        return refuse(reply, 20_001, "invalid param");
    };
    let form: BTreeMap<String, String> = url::form_urlencoded::parse(body)
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    let kind: u8 = form
        .get("tx_type")
        .and_then(|kind| kind.parse().ok())
        .unwrap_or(0);
    let info: Value = form
        .get("tx_info")
        .and_then(|info| serde_json::from_str(info).ok())
        .unwrap_or_default();
    let mut books = stub.books();
    if let Some(why) = books.refusing.take() {
        return refuse(reply, 21_104, why);
    }
    let index = u8::try_from(info["ApiKeyIndex"].as_u64().unwrap_or(999)).unwrap_or(255);
    if info["AccountIndex"] != INDEX || (kind != 8 && !books.keys.contains_key(&index)) {
        return refuse(reply, 21_109, "api key not found");
    }
    let expected = books.nonces.get(&index).copied().unwrap_or(0);
    if info["Nonce"] != expected {
        return refuse(reply, 21_104, "invalid nonce");
    }
    books.nonces.insert(index, expected + 1);
    match kind {
        8 => {
            let public = unbase64(info["PubKey"].as_str().unwrap_or_default());
            books.keys.insert(index, hex::encode(public));
        }
        14 | 28 => {
            let orders = match kind {
                14 => vec![info.clone()],
                _ => info["Orders"].as_array().cloned().unwrap_or_default(),
            };
            // The first of a group that opens is what its exits wait for.
            let leads = kind == 28 && info["GroupingType"] != 2;
            let mut parent = None;
            for (place, order) in orders.iter().enumerate() {
                // What trades at once and is gone does not rest.
                if order["Type"] == 1 {
                    continue;
                }
                books.last += 1;
                let index = 281_474_976_710_000 + books.last;
                let waits_for = if place > 0 { parent } else { None };
                books.orders.push(listed(order, index, waits_for));
                if place == 0 && leads {
                    parent = Some(index);
                }
            }
        }
        15 => {
            let cancelled = info["Index"].clone();
            books
                .orders
                .retain(|order| order["order_index"] != cancelled);
        }
        20 => {
            let fraction = info["InitialMarginFraction"].as_u64().unwrap_or_default() as f64;
            books.setting = Some((format!("{:.2}", fraction / 100.0), info["MarginMode"] == 1));
        }
        _ => return refuse(reply, 20_001, "invalid param"),
    }
    books.taken.push((kind, info));
    let taken = json!({ "code": 200, "tx_hash": "00".repeat(40),
                        "predicted_execution_time_ms": 1_795_342_103_859_u64 });
    answer(reply, 200, &taken);
}

/// The bytes `text` is in base64: what [`super::tx`] writes, read back.
fn unbase64(text: &str) -> Vec<u8> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let sixes: Vec<u32> = text
        .bytes()
        .filter_map(|digit| ALPHABET.iter().position(|known| *known == digit))
        .map(|six| six as u32)
        .collect();
    let mut bytes = Vec::new();
    for four in sixes.chunks(4) {
        let joined = four.iter().fold(0, |joined, six| joined << 6 | six) << (6 * (4 - four.len()));
        let whole = [(joined >> 16) as u8, (joined >> 8) as u8, joined as u8];
        bytes.extend(&whole[..four.len() - 1]);
    }
    bytes
}

/// What a press of one of the harness's controls does: something of
/// trading's, whose answer is said under the control's name.
type Act = Rc<dyn Fn() -> Doing<String>>;

/// The app's data services over this app's stub ([`Stub::of_this_app`]) and
/// a stand-in wallet that shares [`ACCOUNT`].
fn services() -> DataServices {
    let stub = Stub::of_this_app();
    let wallets = StandIn::new(Reach::Page, &["Rabby Wallet"]);
    assert_eq!(format!("{:#x}", wallets.address()), ACCOUNT);
    DataServices::provide_with(DataConfig {
        ferris: FerrisConfig {
            http_base: "https://ferris.test".to_owned(),
            ws_url: "ws://127.0.0.1:9/v1/ws".to_owned(),
            ..FerrisConfig::production()
        },
        hyperliquid: HyperliquidConfig {
            http_base: "https://hyperliquid.test".to_owned(),
            transport: Some(ScriptedSocket::new(|_| Vec::new()).transport()),
            ..HyperliquidConfig::production()
        },
        lighter: LighterConfig {
            http_base: LIGHTER.to_owned(),
            transport: Some(stub.socket.transport()),
            ..LighterConfig::production()
        },
        wallet: WalletConfig {
            connector: Some(Rc::new(wallets)),
        },
        ..DataConfig::default()
    })
}

/// A workspace of one Trade Panel set to trade on Lighter and to send at
/// the first press, over [`services`]: the widget itself, as a user has it.
#[component]
fn Panel() -> impl IntoElement {
    let services = services();
    // What the account widgets read of the shell.
    provide_context(BarClock::restore());
    AddressGroups::provide();
    let mut stored = StoredWorkspace::default();
    layout::normalize(&mut stored);
    layout::spawn(&mut stored, WidgetKind::TradePanel);
    let widget = layout::widget_ids(&stored)
        .into_iter()
        .next()
        .expect("the widget just added");
    let saved = json!({ "venue": VENUE, "confirmOrders": false });
    let settings = WidgetSettings::read(WidgetKind::TradePanel, saved);
    assert!(layout::set_tab_settings(&mut stored, widget, &settings));
    let stored = signal(stored);
    let picker = Picker::new();
    services
        .sessions
        .follow(move || layout::widget_ids(&stored.get()));
    view()
        .size_full()
        .flex_col()
        .child(layout::workspace(stored, picker))
        .child(picker.view(stored))
}

/// [`harness`], watching the account's open orders alone.
#[component]
fn Desk() -> impl IntoElement {
    harness(false)
}

/// [`harness`], watching the account's margin too, as a widget that trades
/// does.
#[component]
fn Margined() -> impl IntoElement {
    harness(true)
}

/// The app's trading with a control for each thing a widget asks of it,
/// the connected account's open orders watched as a widget watches them
/// (and how many times they were read), its margin too when `margined`, a
/// line saying where the account stands with Lighter, a line saying what
/// the last thing asked answered, and a line saying what was last heard of
/// the account followed.
fn harness(margined: bool) -> impl IntoElement {
    let services = services();
    let (wallet, trading) = (services.wallet.clone(), services.trading.clone());
    let venue: Rc<dyn TradingVenue> = trading.venue(VENUE).expect("Lighter is traded on").clone();
    let said = signal(String::new());
    let heard = signal(String::new());
    // The hold on the account followed, let go with the app as a widget's
    // session lets go of its own.
    let following: Rc<RefCell<Option<Following>>> = Rc::default();
    cleanup({
        let following = following.clone();
        move || drop(following.borrow_mut().take())
    });

    let done = |made: Result<(), crate::data::net::ApiError>| match made {
        Ok(()) => "done".to_owned(),
        Err(error) => format!("refused ({}): {}", error.code, error.message),
    };
    let place = |order: OrderRequest| -> Act {
        let trading = trading.clone();
        Rc::new(move || {
            let placing = trading.place(VENUE, order.clone());
            Box::pin(async move {
                Ok(match placing.await {
                    Ok(placed) => format!("{placed:?}"),
                    Err(error) => format!("refused ({}): {}", error.code, error.message),
                })
            })
        })
    };
    let limit = |size: f64, exits: Exits| OrderRequest {
        instrument: "BTC".to_owned(),
        side: Side::Buy,
        size,
        kind: OrderKind::Limit {
            price: 61_000.5,
            time_in_force: TimeInForce::GoodTilCancelled,
        },
        reduce_only: false,
        exits,
    };
    let both = Exits {
        take_profit: Some(65_000.0),
        stop_loss: Some(58_000.0),
    };
    let stop = Exits {
        take_profit: None,
        stop_loss: Some(58_000.0),
    };
    let closing = OrderRequest::closing("BTC", crate::data::portfolio::Direction::Long, 0.001);

    // The connected account's open orders, watched while it is connected.
    let orders = signal(None::<ReadSignal<QueryState<OpenOrders>>>);
    {
        let trading = trading.clone();
        let watched: Rc<RefCell<Option<Watch<OpenOrders>>>> = Rc::default();
        effect(move || {
            let account = trading.account(VENUE);
            untracked(|| {
                let watch = account.and_then(|account| trading.watch_open_orders(&account).ok());
                orders.set(watch.as_ref().map(Watch::state));
                *watched.borrow_mut() = watch;
            });
        });
    }
    // How many times they were read: each answer, and each time the stream
    // or a transaction had them read again.
    let reads = signal(0_u32);
    let last_read = Cell::new(None::<f64>);
    effect(move || {
        let read_at = orders
            .get()
            .and_then(|state| state.with(|state| state.updated_at_ms));
        if read_at.is_some() && last_read.replace(read_at) != read_at {
            untracked(|| reads.set(reads.get_untracked() + 1));
        }
    });
    // Its margin, watched the same way and counted the same way.
    let margin = signal(None::<ReadSignal<QueryState<MarginAccount>>>);
    if margined {
        let trading = trading.clone();
        let watched: Rc<RefCell<Option<Watch<MarginAccount>>>> = Rc::default();
        effect(move || {
            let account = trading.account(VENUE);
            untracked(|| {
                let watch = account.and_then(|account| trading.watch_margin(&account).ok());
                margin.set(watch.as_ref().map(Watch::state));
                *watched.borrow_mut() = watch;
            });
        });
    }
    let margin_reads = signal(0_u32);
    let last_margin = Cell::new(None::<f64>);
    effect(move || {
        let read_at = margin
            .get()
            .and_then(|state| state.with(|state| state.updated_at_ms));
        if read_at.is_some() && last_margin.replace(read_at) != read_at {
            untracked(|| margin_reads.set(margin_reads.get_untracked() + 1));
        }
    });
    let open = move || {
        let read = orders
            .get()
            .and_then(|state| state.with(|state| state.data.clone()));
        let Some(read) = read else {
            return "orders open: unread".to_owned();
        };
        let lines: Vec<String> = read
            .0
            .iter()
            .map(|order| {
                let exits = (order.exits.take_profit, order.exits.stop_loss);
                format!(
                    "{} {} at {:?} with {exits:?}",
                    order.kind, order.id, order.price
                )
            })
            .collect();
        format!("orders open: {} [{}]", lines.len(), lines.join("; "))
    };
    let cancel: Act = {
        let (venue, trading) = (venue.clone(), trading.clone());
        Rc::new(move || {
            let (hub, spec) = (venue.queries().clone(), venue.open_orders_query(ACCOUNT));
            let trading = trading.clone();
            Box::pin(async move {
                let orders = hub.get(spec).await?;
                let first = orders.0.first().expect("an order to cancel");
                let cancel = CancelRequest {
                    instrument: first.instrument.clone(),
                    id: first.id.clone(),
                };
                Ok(match trading.cancel(VENUE, cancel).await {
                    Ok(()) => format!("cancelled {}", first.id),
                    Err(error) => format!("refused ({}): {}", error.code, error.message),
                })
            })
        })
    };
    let leverage: Act = {
        let trading = trading.clone();
        Rc::new(move || {
            let setting = Leverage {
                value: 20.0,
                mode: MarginMode::Isolated,
            };
            let set = trading.set_leverage(VENUE, "BTC", setting);
            Box::pin(async move { Ok(done(set.await)) })
        })
    };
    let setting: Act = {
        let venue = venue.clone();
        Rc::new(move || {
            let spec = venue
                .leverage_query(ACCOUNT, "BTC")
                .expect("Lighter has a setting");
            let hub = venue.queries().clone();
            Box::pin(async move { Ok(format!("{:?}", *hub.get(spec).await?)) })
        })
    };
    let protect: Act = {
        let trading = trading.clone();
        Rc::new(move || {
            let protecting = trading.protect(VENUE, "BTC", stop);
            Box::pin(async move { Ok(done(protecting.await)) })
        })
    };
    let acts: Vec<(&'static str, Act)> = vec![
        ("buy with exits", place(limit(0.0015, both))),
        ("buy with a stop", place(limit(0.0015, stop))),
        ("buy too finely", place(limit(0.000_001, Exits::default()))),
        ("buy too little", place(limit(0.0001, Exits::default()))),
        ("close at market", place(closing)),
        ("cancel the first", cancel),
        ("set the leverage", leverage),
        ("read the setting", setting),
        ("protect the position", protect),
    ];

    let control = |label: &'static str, act: Rc<dyn Fn()>| {
        view()
            .role(AccessibilityRole::Button)
            .label(label)
            .on_press(move || act())
            .child(text(label))
    };
    let asked = acts.into_iter().map(|(name, act)| {
        control(
            name,
            Rc::new(move || {
                let doing = act();
                spawn(async move {
                    let answered = match doing.await {
                        Ok(answered) => answered,
                        Err(error) => format!("failed ({}): {}", error.code, error.message),
                    };
                    said.try_set(format!("{name}: {answered}"));
                });
            }),
        )
    });
    let (connects, enables, disables, follows, stands, whose) = (
        wallet.clone(),
        trading.clone(),
        trading.clone(),
        trading.clone(),
        trading.clone(),
        trading.clone(),
    );
    view()
        .size_full()
        .flex_col()
        .child(control(
            "the wallet connects",
            Rc::new(move || connects.connect()),
        ))
        .child(control(
            "enable trading",
            Rc::new(move || enables.enable(VENUE)),
        ))
        .child(control(
            "disable trading",
            Rc::new(move || disables.disable(VENUE)),
        ))
        .child(control(
            "follow the account",
            Rc::new(move || {
                let account = follows.account(VENUE).expect("a connected account");
                let hears = move |event: &AccountEvent| {
                    let AccountEvent::Filled(fill) = event;
                    let traded = (fill.side, fill.size, &fill.instrument, fill.price);
                    heard.try_set(format!("{traded:?}"));
                };
                *following.borrow_mut() = follows.follow(&account, hears).ok();
            }),
        ))
        .children(asked)
        .child(text(move || {
            let account = whose.account(VENUE).map(|account| account.address);
            format!("account {}", account.unwrap_or_else(|| "none".to_owned()))
        }))
        .child(text(move || {
            format!("lighter is {:?}", stands.access(VENUE))
        }))
        .child(text(open))
        .child(text(move || format!("orders read: {}", reads.get())))
        .child(text(move || format!("margin read: {}", margin_reads.get())))
        .child(text(move || format!("said {}", said.get())))
        .child(text(move || format!("heard {}", heard.get())))
}

/// A started app, with what a test reads of it and does to it.
struct Bench {
    app: Headless,
    network: Arc<StubNetwork>,
    stub: Stub,
    storage: Arc<MemoryStorage>,
}

impl Bench {
    /// The app over a new stub and storage that holds `saved`.
    fn start(saved: BTreeMap<String, String>) -> Self {
        Self::start_over(&Stub::new(), saved)
    }

    /// The app over `stub` and storage that holds `saved`.
    fn start_over(stub: &Stub, saved: BTreeMap<String, String>) -> Self {
        Self::start_rooted(nsh_testing::windowed_root::<Desk>(), stub, saved)
    }

    /// The app rooted at `root` over `stub` and storage that holds `saved`.
    fn start_rooted(root: Root, stub: &Stub, mut saved: BTreeMap<String, String>) -> Self {
        let (asked, sent) = (stub.clone(), stub.clone());
        let network = Arc::new(StubNetwork::new([
            (
                format!("GET {LIGHTER}/api/v1/*"),
                StubRoute::Handler(Arc::new(move |request: HttpRequest, reply: StubReply| {
                    stub_get(&asked, &request, reply);
                })),
            ),
            (
                format!("POST {LIGHTER}/api/v1/sendTx"),
                StubRoute::Handler(Arc::new(move |request: HttpRequest, reply: StubReply| {
                    stub_send(&sent, &request, reply);
                })),
            ),
        ]));
        saved.insert(RENDEZVOUS.to_owned(), stub.id.clone());
        let storage = MemoryStorage::new(saved);
        let mut config = EngineConfig::new(root);
        config.viewport = Size::new(px(1280.0), px(800.0));
        config.storage = Some(storage.clone());
        config.network = Some(network.clone());
        config.environment.os = Os::Windows;
        config.environment.idiom = Idiom::Desktop;
        Self {
            app: Headless::start(config).expect("the app starts"),
            network,
            stub: stub.clone(),
            storage,
        }
    }

    /// Connects the wallet.
    fn connect(&mut self) {
        self.app.wait_for("account none").unwrap();
        self.app.press("the wallet connects").unwrap();
        self.app.wait_for(&format!("account {ACCOUNT}")).unwrap();
    }

    /// Enables trading, and waits for the orders the key can read: read
    /// once as none before there was a key, and once with it.
    fn enable(&mut self) {
        self.app.press("enable trading").unwrap();
        self.app.wait_for("lighter is Ready").unwrap();
        self.app.wait_for("orders read: 2").unwrap();
    }

    /// The line on screen that starts with `prefix`, without it.
    fn line(&self, prefix: &str) -> Option<String> {
        let texts = self.app.screen().texts().into_iter();
        let mut lines = texts.filter_map(|text| text.strip_prefix(prefix).map(str::to_owned));
        lines.next()
    }

    /// How many times the account's open orders were read so far.
    fn reads(&self) -> u32 {
        let read = self.line("orders read: ").expect("the orders' count shows");
        read.parse().expect(&read)
    }

    /// Waits for what a transaction changed to have been asked for again,
    /// which is a moment after it: the open orders are read once more than
    /// the `before` times they were, after the account's accounts.
    fn settle(&mut self, before: u32) {
        let more = |screen: &nsh_testing::Screen| {
            let texts = screen.texts().into_iter();
            let mut read =
                texts.filter_map(|text| text.strip_prefix("orders read: ")?.parse().ok());
            read.next().is_some_and(|read: u32| read > before)
        };
        self.app
            .wait_until("it being asked for again", Duration::from_secs(15), more)
            .unwrap();
    }

    /// The account's open orders, as they show.
    fn orders(&self) -> String {
        self.line("orders open: ").expect("the orders show")
    }

    /// Presses the control named `name` and waits for what it answered.
    fn ask(&mut self, name: &str) -> String {
        self.app.press(name).unwrap();
        let prefix = format!("said {name}: ");
        let answered = RefCell::new(String::new());
        self.app
            .wait_until("its answer", WAIT, |screen| {
                let said = screen
                    .texts()
                    .into_iter()
                    .find(|text| text.starts_with(&prefix));
                said.is_some_and(|said| {
                    said[prefix.len()..].clone_into(&mut answered.borrow_mut());
                    true
                })
            })
            .unwrap();
        answered.into_inner()
    }

    /// The paths and queries of the requests made of Lighter so far, in
    /// order, a transaction posted being `POST`.
    fn asked(&self) -> Vec<String> {
        let requests = self.network.requests().into_iter();
        requests
            .filter_map(|request| {
                let path = request.url.strip_prefix(LIGHTER)?;
                Some(match request.method.as_str() {
                    "POST" => "POST".to_owned(),
                    _ => path.trim_start_matches("/api/v1/").to_owned(),
                })
            })
            .collect()
    }

    /// How many requests so far start with `path`.
    fn count(&self, path: &str) -> usize {
        let asked = self.asked();
        asked.iter().filter(|asked| asked.starts_with(path)).count()
    }

    /// What the app keeps for the account between runs.
    fn kept(&self) -> Value {
        let saved = self.storage.entries();
        let kept = saved
            .get(KEPT)
            .map(|kept| serde_json::from_str(kept).unwrap());
        kept.map_or(Value::Null, |kept: Value| kept["Testnet"][ACCOUNT].clone())
    }

    /// The key the app keeps for the account.
    fn key(&self) -> SecretKey {
        let kept = self.kept();
        let mut bytes = [0; 40];
        hex::decode_to_slice(kept["key"].as_str().expect("a key is kept"), &mut bytes).unwrap();
        SecretKey::from_bytes(&bytes).expect("what is kept is a key")
    }
}

/// An order of a transaction's JSON, as the transaction was made of it.
fn order_of(order: &Value) -> Order {
    let number = |name: &str| order[name].as_u64().expect(name);
    Order {
        market: number("MarketIndex"),
        client_index: number("ClientOrderIndex"),
        base_amount: number("BaseAmount"),
        price: u32::try_from(number("Price")).unwrap(),
        ask: number("IsAsk") == 1,
        kind: match number("Type") {
            0 => OrderType::Limit,
            1 => OrderType::Market,
            2 => OrderType::StopLoss,
            4 => OrderType::TakeProfit,
            other => panic!("an order of type {other}"),
        },
        standing: match number("TimeInForce") {
            0 => Standing::ImmediateOrCancel,
            1 => Standing::GoodTillTime,
            2 => Standing::PostOnly,
            other => panic!("an order standing {other}"),
        },
        reduce_only: number("ReduceOnly") == 1,
        trigger_price: u32::try_from(number("TriggerPrice")).unwrap(),
        expires_at: number("OrderExpiry"),
    }
}

/// Whether the transaction of the kind numbered `kind` that was posted as
/// `info` is signed by `key` for the stubbed Lighter's net, as Lighter
/// checks it: its JSON read back into what was hashed.
fn signed_by(key: &SecretKey, kind: u8, info: &Value) -> bool {
    let number = |name: &str| info[name].as_u64().expect(name);
    let transaction = match kind {
        8 => Transaction::ChangePubKey {
            key: key.public(),
            wallet_signature: info["L1Sig"].as_str().unwrap().to_owned(),
        },
        14 => Transaction::CreateOrder(order_of(info)),
        28 => Transaction::CreateGroupedOrders {
            grouping: match number("GroupingType") {
                1 => Grouping::WithExit,
                2 => Grouping::Exits,
                _ => Grouping::WithExits,
            },
            orders: info["Orders"]
                .as_array()
                .unwrap()
                .iter()
                .map(order_of)
                .collect(),
        },
        15 => Transaction::CancelOrder {
            market: number("MarketIndex"),
            order: number("Index"),
        },
        20 => Transaction::UpdateLeverage {
            market: number("MarketIndex"),
            margin_fraction: u16::try_from(number("InitialMarginFraction")).unwrap(),
            isolated: number("MarginMode") == 1,
        },
        other => panic!("a transaction of kind {other}"),
    };
    assert_eq!(transaction.kind(), kind);
    let envelope = Envelope {
        network: Network::Testnet,
        account: number("AccountIndex"),
        key_index: u8::try_from(number("ApiKeyIndex")).unwrap(),
        nonce: number("Nonce"),
        expires_at: number("ExpiredAt"),
    };
    let signature = unbase64(info["Sig"].as_str().unwrap());
    let signature: [u8; 80] = signature.try_into().expect("a signature is eighty bytes");
    verifies(key.public(), transaction.hash(&envelope), &signature)
}

#[test]
fn an_account_registers_a_key_and_trades_with_it() {
    let mut bench = Bench::start(BTreeMap::new());
    bench.connect();
    bench.app.wait_for("lighter is Locked").unwrap();
    // Nothing is asked of Lighter for a wallet that connected: no key is
    // kept, and the open orders are told only to one.
    bench.app.wait_for("orders open: 0 []").unwrap();
    assert!(bench.asked().is_empty(), "{:?}", bench.asked());

    // Enabling: which account, which indexes are taken, the new key's
    // nonce, the registration, and Lighter listing it; then the orders
    // only the key can read.
    bench.enable();
    assert_eq!(
        bench.asked(),
        [
            format!("account?by=l1_address&value={ACCOUNT}"),
            format!("apikeys?account_index={INDEX}&api_key_index=255"),
            format!("nextNonce?account_index={INDEX}&api_key_index={KEY_INDEX}"),
            "POST".to_owned(),
            format!("apikeys?account_index={INDEX}&api_key_index={KEY_INDEX}"),
            format!("accountActiveOrders?account_index={INDEX}"),
        ]
    );
    // The key is kept under the net and the address, at the index nothing
    // was registered at, and it is the one Lighter lists there now.
    let kept = bench.kept();
    assert_eq!(
        (&kept["account"], &kept["index"]),
        (&json!(INDEX), &json!(KEY_INDEX))
    );
    let key = bench.key();
    assert_eq!(kept["public"], key.public().to_string());
    assert_eq!(
        bench.stub.books().keys[&KEY_INDEX],
        key.public().to_string()
    );
    // The others' keys are as they were.
    assert_eq!(bench.stub.books().keys.len(), 4);

    // The registration: signed by the new key, and by the account's wallet
    // over the line of text that names the key.
    let registered = bench.stub.taken(8);
    assert_eq!(registered.len(), 1);
    let registration = &registered[0];
    assert_eq!(registration["Nonce"], 0);
    assert_eq!(registration["L2TxAttributes"], Value::Null);
    assert!(signed_by(&key, 8, registration));
    let envelope = Envelope {
        network: Network::Testnet,
        account: INDEX,
        key_index: KEY_INDEX,
        nonce: 0,
        expires_at: 0,
    };
    let by_wallet: Signature = registration["L1Sig"].as_str().unwrap().parse().unwrap();
    let message = register_message(key.public(), &envelope);
    assert_eq!(
        by_wallet.recover_address_from_msg(message.as_bytes()).ok(),
        Some(ACCOUNT.parse::<Address>().unwrap())
    );

    // An order with both its exits: one group, in the market's integers,
    // with the key's next nonce, which is asked for once.
    let read = bench.reads();
    assert_eq!(bench.ask("buy with exits"), "Accepted");
    assert_eq!(bench.count("orderBookDetails"), 1);
    assert_eq!(bench.count("nextNonce"), 2);
    let groups = bench.stub.taken(28);
    let group = &groups[0];
    assert!(signed_by(&key, 28, group));
    assert_eq!(
        (&group["Nonce"], &group["GroupingType"]),
        (&json!(1), &json!(3))
    );
    let legs: Vec<Order> = group["Orders"]
        .as_array()
        .unwrap()
        .iter()
        .map(order_of)
        .collect();
    assert_eq!(legs.len(), 3);
    let (order, take_profit, stop_loss) = (legs[0], legs[1], legs[2]);
    assert_eq!(
        (order.market, order.base_amount, order.price),
        (1, 150, 610_005)
    );
    assert_eq!(
        (order.kind, order.standing),
        (OrderType::Limit, Standing::GoodTillTime)
    );
    assert!(!order.ask && !order.reduce_only);
    assert_eq!(
        (
            take_profit.kind,
            take_profit.trigger_price,
            take_profit.price
        ),
        (OrderType::TakeProfit, 650_000, 585_000)
    );
    assert_eq!(
        (stop_loss.kind, stop_loss.trigger_price, stop_loss.price),
        (OrderType::StopLoss, 580_000, 522_000)
    );
    for exit in [take_profit, stop_loss] {
        assert!(exit.ask && exit.reduce_only);
        assert_eq!(
            (exit.base_amount, exit.standing),
            (0, Standing::ImmediateOrCancel)
        );
        // They rest as long as what they wait for does: 28 days.
        assert_eq!(exit.expires_at, order.expires_at);
    }
    let lasts = order.expires_at - group["ExpiredAt"].as_u64().unwrap();
    assert!(
        (27 * 86_400_000..=28 * 86_400_000).contains(&lasts),
        "{lasts}"
    );

    // What a transaction changed is asked for a moment after it. The open
    // orders are asked for with a token of the key: one order, with its
    // exits waiting for it.
    bench.settle(read);
    let orders = bench.orders();
    assert!(
        orders.starts_with("1 [Limit 281474976710001 at Some(61000.5)"),
        "{orders}"
    );
    assert!(
        orders.ends_with("with (Some(65000.0), Some(58000.0))]"),
        "{orders}"
    );
    let token = bench.stub.books().tokens.last().cloned().expect("a token");
    let parts: Vec<&str> = token.split(':').collect();
    assert_eq!((parts[1], parts[2]), ("2180", "253"));
    let said = token.rsplit_once(':').unwrap().0;
    let mut signature = [0; 80];
    hex::decode_to_slice(parts[3], &mut signature).unwrap();
    assert!(verifies(
        key.public(),
        super::crypto::Members::text(said).hash(),
        &signature
    ));

    // Closing at the market's price: bounded from the middle of the book,
    // only reducing, and gone if it does not trade. The nonce is counted
    // here from the last one.
    assert_eq!(bench.ask("close at market"), "Accepted");
    assert_eq!(bench.count("orderBookOrders?market_id=1&limit=1"), 1);
    assert_eq!(bench.count("nextNonce"), 2);
    let closing = &bench.stub.taken(14)[0];
    assert!(signed_by(&key, 14, closing));
    let closing = order_of(closing);
    assert_eq!(bench.stub.taken(14)[0]["Nonce"], 2);
    assert_eq!(
        (closing.kind, closing.standing),
        (OrderType::Market, Standing::ImmediateOrCancel)
    );
    // 5% under 61,000, the middle of 60,999.5 and 61,000.5.
    assert_eq!((closing.base_amount, closing.price), (100, 579_500));
    assert!(closing.ask && closing.reduce_only);
    assert_eq!((closing.expires_at, closing.trigger_price), (0, 0));

    // What cannot be an order is refused in words, and nothing is sent.
    let posted = bench.count("POST");
    assert_eq!(
        bench.ask("buy too finely"),
        "refused (INVALID_REQUEST): A size of BTC has 5 decimal places at most."
    );
    assert_eq!(
        bench.ask("buy too little"),
        "refused (INVALID_REQUEST): An order on Lighter is worth $10 at least; this one is \
         worth about $6.10."
    );
    assert_eq!(bench.count("POST"), posted);

    // What Lighter refuses is said as it said it, and has the next
    // transaction ask what its nonce is.
    bench.stub.books().refusing = Some("invalid nonce");
    assert_eq!(
        bench.ask("set the leverage"),
        "refused (21104): invalid nonce"
    );
    assert_eq!(bench.count("nextNonce"), 2);
    let read = bench.reads();
    assert_eq!(bench.ask("set the leverage"), "done");
    assert_eq!(bench.count("nextNonce"), 3);
    let leverage = &bench.stub.taken(20)[0];
    assert!(signed_by(&key, 20, leverage));
    assert_eq!(
        (
            &leverage["MarketIndex"],
            &leverage["InitialMarginFraction"],
            &leverage["MarginMode"]
        ),
        (&json!(1), &json!(500), &json!(1))
    );
    // The setting is read from the account's own entry for the market.
    bench.settle(read);
    assert_eq!(
        bench.ask("read the setting"),
        "Some(Leverage { value: 20.0, mode: Isolated })"
    );

    // A stop for the position held: an order of its own, of the whole
    // position.
    let read = bench.reads();
    assert_eq!(bench.ask("protect the position"), "done");
    let stop = order_of(&bench.stub.taken(14)[1]);
    assert!(signed_by(&key, 14, &bench.stub.taken(14)[1]));
    assert_eq!(
        (stop.kind, stop.trigger_price, stop.price),
        (OrderType::StopLoss, 580_000, 522_000)
    );
    assert!(stop.ask && stop.reduce_only && stop.base_amount == 0);

    // A cancel names the order by its index.
    bench.settle(read);
    let cancelled = bench.ask("cancel the first");
    let index: u64 = cancelled
        .strip_prefix("cancelled ")
        .expect(&cancelled)
        .parse()
        .unwrap();
    let cancel = &bench.stub.taken(15)[0];
    assert!(signed_by(&key, 15, cancel));
    assert_eq!(
        (&cancel["MarketIndex"], &cancel["Index"]),
        (&json!(1), &json!(index))
    );

    // Followed live: its positions and its trades, and its orders with a
    // token (subscribed in the order of their names).
    bench.app.press("follow the account").unwrap();
    let socket = bench.stub.socket.clone();
    let subscribed = move || -> Vec<Value> {
        let frames = socket.sent().into_iter();
        let read = frames.filter_map(|frame| serde_json::from_str::<Value>(&frame).ok());
        read.filter(|frame| frame["type"] == "subscribe").collect()
    };
    bench
        .app
        .wait_until("the account being followed", WAIT, |_| {
            subscribed().len() >= 3
        })
        .unwrap();
    let channels: Vec<(String, bool)> = subscribed()
        .iter()
        .map(|frame| {
            (
                frame["channel"].as_str().unwrap().to_owned(),
                frame["auth"].is_string(),
            )
        })
        .collect();
    assert_eq!(
        channels,
        [
            ("account_all_orders/2180".to_owned(), true),
            ("account_all_positions/2180".to_owned(), false),
            ("account_all_trades/2180".to_owned(), false),
        ]
    );
}

#[test]
fn a_stop_alone_waits_for_its_order() {
    let mut bench = Bench::start(BTreeMap::new());
    bench.connect();
    bench.enable();
    let read = bench.reads();
    assert_eq!(bench.ask("buy with a stop"), "Accepted");
    let group = &bench.stub.taken(28)[0];
    assert!(signed_by(&bench.key(), 28, group));
    // An order and its one exit.
    assert_eq!(group["GroupingType"], 1);
    let legs: Vec<Order> = group["Orders"]
        .as_array()
        .unwrap()
        .iter()
        .map(order_of)
        .collect();
    assert_eq!(legs.len(), 2);
    assert_eq!(legs[1].kind, OrderType::StopLoss);
    bench.settle(read);
    let orders = bench.orders();
    assert!(orders.ends_with("with (None, Some(58000.0))]"), "{orders}");
}

#[test]
fn trading_stays_enabled_between_runs_while_lighter_lists_the_key() {
    let stub = Stub::new();
    let mut first = Bench::start_over(&stub, BTreeMap::new());
    first.connect();
    first.app.press("enable trading").unwrap();
    first.app.wait_for("lighter is Ready").unwrap();
    let saved = first.storage.entries();
    let public = first.key().public().to_string();
    drop(first.app);

    // The next run takes the key up once Lighter lists it: no signature,
    // and nothing registered.
    let kept = || BTreeMap::from([(KEPT.to_owned(), saved[KEPT].clone())]);
    let mut second = Bench::start_over(&stub, kept());
    second.connect();
    second.app.wait_for("lighter is Ready").unwrap();
    second.app.wait_for("orders read: 2").unwrap();
    assert_eq!(
        second.asked(),
        [
            format!("apikeys?account_index={INDEX}&api_key_index={KEY_INDEX}"),
            // The orders only the key can read, once it is taken up.
            format!("accountActiveOrders?account_index={INDEX}"),
        ]
    );
    assert_eq!(stub.taken(8).len(), 1);
    assert_eq!(second.ask("buy with a stop"), "Accepted");

    // Disabling forgets the key and remembers where it was.
    second.app.press("disable trading").unwrap();
    second.app.wait_for("lighter is Locked").unwrap();
    let retired = second.kept();
    assert_eq!(retired["key"], Value::Null);
    assert_eq!(
        (&retired["index"], &retired["public"]),
        (&json!(KEY_INDEX), &json!(public))
    );
    let refused = second.ask("buy with a stop");
    assert!(refused.contains("Enable trading on Lighter"), "{refused}");
    // Enabling again registers another key at the same index.
    second.app.press("enable trading").unwrap();
    second.app.wait_for("lighter is Ready").unwrap();
    assert_eq!(stub.taken(8).len(), 2);
    assert_eq!(stub.taken(8)[1]["ApiKeyIndex"], KEY_INDEX);
    assert_ne!(second.key().public().to_string(), public);
    assert_eq!(stub.books().keys.len(), 4);
    let renewed = second.storage.entries();
    drop(second.app);

    // A key Lighter no longer lists at its index is discarded: somebody
    // registered another there.
    stub.books().keys.insert(KEY_INDEX, "dd".repeat(40));
    let kept = BTreeMap::from([(KEPT.to_owned(), renewed[KEPT].clone())]);
    let mut third = Bench::start_over(&stub, kept);
    third.connect();
    third.app.wait_for("lighter is Locked").unwrap();
    third
        .app
        .wait_until("the key being discarded", WAIT, |_| {
            !third.storage.entries().contains_key(KEPT)
        })
        .unwrap();
    // Enabling then takes an index that is nobody's, and leaves theirs.
    third.app.press("enable trading").unwrap();
    third.app.wait_for("lighter is Ready").unwrap();
    assert_eq!(third.kept()["index"], 252);
    assert_eq!(stub.books().keys[&KEY_INDEX], "dd".repeat(40));
}

#[test]
fn what_the_market_does_to_the_account_shows_as_it_happens() {
    let stub = Stub::new();
    let mut bench = Bench::start_rooted(
        nsh_testing::windowed_root::<Margined>(),
        &stub,
        BTreeMap::new(),
    );
    bench.connect();
    bench.enable();
    bench.app.press("follow the account").unwrap();
    let socket = stub.socket.clone();
    let subscribed = move || {
        let subscribes = socket.sent().into_iter();
        subscribes
            .filter(|frame| frame.contains(r#""subscribe""#))
            .count()
    };
    bench
        .app
        .wait_until("the account being followed", WAIT, |_| subscribed() >= 3)
        .unwrap();
    let asked = bench.count("accountActiveOrders");

    // An order comes to rest that this app did not place: its stream says
    // so, and it shows at once with nothing asked for.
    let order = |status: &str| {
        json!({
            "channel": format!("account_all_orders:{INDEX}"),
            "orders": { "1": [{
                "order_index": 900, "order_id": "900", "market_index": 1,
                "market_symbol": "BTC", "initial_base_amount": "0.00200",
                "remaining_base_amount": "0.00200", "price": "60000.0", "is_ask": false,
                "type": "limit", "reduce_only": false, "trigger_price": "0.0",
                "status": status, "trigger_status": "na", "parent_order_index": 0,
                "timestamp": 1_795_000_900,
            }] },
            "type": "update/account_all_orders",
        })
        .to_string()
    };
    stub.socket.push(order("open"));
    bench
        .app
        .wait_for("orders open: 1 [Limit 900 at Some(60000.0) with (None, None)]")
        .unwrap();
    // It trades: it is over, and gone.
    stub.socket.push(order("filled"));
    bench.app.wait_for("orders open: 0 []").unwrap();
    assert_eq!(bench.count("accountActiveOrders"), asked);

    // Its positions changed: its accounts are asked for again a moment
    // later, and whoever watches its margin has it anew.
    let margin_read = bench.line("margin read: ").unwrap();
    let accounts_asked = bench.count("account?");
    let positions = json!({
        "channel": format!("account_all_positions:{INDEX}"),
        "positions": { "1": { "market_id": 1, "symbol": "BTC", "sign": 1,
                              "position": "0.01200", "avg_entry_price": "60166.7" } },
        "type": "update/account_all_positions",
    });
    stub.socket.push(positions.to_string());
    bench
        .app
        .wait_until(
            "its margin being read again",
            Duration::from_secs(10),
            |screen| {
                let texts = screen.texts().into_iter();
                let mut read =
                    texts.filter_map(|text| Some(text.strip_prefix("margin read: ")?.to_owned()));
                read.next().is_some_and(|read| read != margin_read)
            },
        )
        .unwrap();
    assert_eq!(bench.count("account?"), accounts_asked + 1);
    // A funding round names no position, and has nothing asked for.
    let funded = json!({
        "channel": format!("account_all_positions:{INDEX}"),
        "last_funding_round": { "1": "0.243092" }, "positions": {},
        "type": "update/account_all_positions",
    });
    stub.socket.push(funded.to_string());

    // A trade of the account's is said as it happens.
    let trade = json!({
        "channel": format!("account_all_trades:{INDEX}"),
        "trades": { "1": [{
            "trade_id": 42_152_504_248_u64, "type": "trade", "market_id": 1,
            "market_symbol": "BTC", "size": "0.00200", "price": "60000.0",
            "usd_amount": "120.000000", "ask_account_id": 52_367, "bid_account_id": INDEX,
            "is_maker_ask": false, "timestamp": 1_795_341_608_548_u64, "maker_fee": 40,
        }] },
        "type": "update/account_all_trades",
    });
    stub.socket.push(trade.to_string());
    bench
        .app
        .wait_for("heard (Buy, 0.002, \"BTC\", 60000.0)")
        .unwrap();
    // What a subscription opens with is what was, and is not said.
    let opened = json!({
        "channel": format!("account_all_trades:{INDEX}"),
        "trades": { "1": [{
            "trade_id": 7, "market_symbol": "BTC", "size": "1.00000", "price": "1.0",
            "ask_account_id": INDEX, "bid_account_id": 9, "timestamp": 1_795_341_000_000_u64,
        }] },
        "type": "subscribed/account_all_trades",
    });
    stub.socket.push(opened.to_string());
    stub.socket.push(order("open"));
    bench
        .app
        .wait_for("orders open: 1 [Limit 900 at Some(60000.0) with (None, None)]")
        .unwrap();
    assert!(bench.app.shows("heard (Buy, 0.002, \"BTC\", 60000.0)"));
}

#[test]
fn the_trade_panel_trades_on_lighter() {
    let stub = Stub::new();
    let mut bench = Bench::start_rooted(
        nsh_testing::windowed_root::<Panel>(),
        &stub,
        BTreeMap::new(),
    );
    let app = &mut bench.app;
    // No wallet, then one that has enabled nothing: what enabling asks is
    // said, in Lighter's terms.
    app.wait_for("id:trade-panel-ask").unwrap();
    app.press("Connect wallet").unwrap();
    app.wait_for("Trading off").unwrap();
    assert!(app.screen().texts().iter().any(|text| {
        text.starts_with("Your wallet signs one message.")
            && text.ends_with("it cannot send funds anywhere else.")
    }));
    assert!(stub.taken(8).is_empty());

    // Enabled: the account's bar and the ticket, over Lighter's reports.
    app.press("label:Enable trading on Lighter").unwrap();
    app.wait_for("id:trade-panel-account").unwrap();
    assert_eq!(stub.taken(8).len(), 1);
    app.wait_for("$1,234.50").unwrap();
    assert!(app.shows("Lighter"));
    app.wait_for("No open orders").unwrap();
    // The market's price is heard (61,012.74, shown to a place), and
    // brought to a price Lighter takes for it: BTC's have one decimal
    // place there.
    app.wait_for("id:trade-panel-mid >> text:61012.7").unwrap();
    // The position it holds shows with it, and how it is set to trade.
    assert!(app.shows("LONG") && app.shows("0.01 BTC") && app.shows("Cross"));
    app.type_text("id:trade-panel-size", "0.0015").unwrap();
    app.press("id:trade-panel-use-mid").unwrap();
    assert_eq!(
        app.screen().input_text("id:trade-panel-price").unwrap(),
        Some("61012.7")
    );

    // Sent: in Lighter's integers, signed by the key the app keeps.
    app.press("id:trade-panel-send").unwrap();
    app.wait_for("Accepted: Buy 0.0015 BTC at 61012.7.")
        .unwrap();
    let placed = stub.taken(14);
    assert_eq!(placed.len(), 1);
    let order = order_of(&placed[0]);
    assert_eq!(
        (order.market, order.base_amount, order.price),
        (1, 150, 610_127)
    );
    assert_eq!(
        (order.kind, order.standing),
        (OrderType::Limit, Standing::GoodTillTime)
    );

    // It shows among the open orders a moment later, and × cancels it.
    app.wait_for("id:trade-panel-order-281474976710001")
        .unwrap();
    app.press("label:Cancel Buy 0.0015 BTC at 61012.7").unwrap();
    app.wait_for("Cancelled: Buy 0.0015 BTC at 61012.7.")
        .unwrap();
    app.wait_for("No open orders").unwrap();
    assert_eq!(stub.taken(15)[0]["Index"], 281_474_976_710_001_u64);
    let key = bench.key();
    assert!(signed_by(&key, 14, &placed[0]) && signed_by(&key, 15, &stub.taken(15)[0]));
}
