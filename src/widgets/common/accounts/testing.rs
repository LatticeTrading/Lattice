//! What the account widgets' tests share: a workspace of one widget over
//! data services whose Hyperliquid is the test's stub network, and that
//! stub's replies.
//!
//! The stubbed Hyperliquid knows three addresses: [`USER`], an account with
//! two positions, three tokens, a history, staked HYPE, a vault share and
//! a fee tier; [`IDLE`], one that never traded; and [`LIMITED`], whose
//! every request is refused with a `429`. The tokens' prices are the same
//! for every account. A fourth, [`SLOW`], is answered [`SLOW_FOR`] late.
//!
//! The stubbed Lighter ([`LIGHTER`]) knows one address, [`TRADER`], which
//! holds a main account, a sub-account that began ten days ago and a pool
//! it runs, with the standing, the positions, the assets, a history and
//! the trades of each; any other address holds no account there, and
//! [`LIMITED`]'s requests are refused there too. Its assets' prices are
//! the same for every address.
//!
//! The stubbed TxFlow ([`TXFLOW`]) knows [`USER`] too: an isolated BTC
//! long, two trades, a day's, a week's and a month's figures, a fee tier and
//! a vault share. Any other address is an empty account there, and
//! [`LIMITED`]'s requests are refused.

use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};

use nsh_core::{
    element::Root,
    platform::{Idiom, Os},
    prelude::*,
    time::{Clock, unix_ms},
};
use nsh_engine::{EngineConfig, HttpBody, HttpRequest};
use nsh_testing::{Headless, MemoryStorage, StubNetwork, StubReply, StubResponse, StubRoute};
use serde_json::{Value, json};

use super::AddressGroups;
use crate::{
    data::{
        DataConfig, DataServices, ferris::FerrisConfig, hyperliquid::HyperliquidConfig,
        lighter::LighterConfig, txflow::TxFlowConfig,
    },
    layout,
    shell::clock::Clock as BarClock,
    storage::StoredWorkspace,
    widgets::{WidgetKind, WidgetSettings, picker::Picker},
};

/// A Hyperliquid no test reaches: its HTTP is the test's stub network.
pub(crate) const HYPERLIQUID: &str = "https://hyperliquid.test";
/// A Lighter no test reaches either.
pub(crate) const LIGHTER: &str = "https://lighter.test";
/// A TxFlow no test reaches either.
pub(crate) const TXFLOW: &str = "https://txflow.test";
/// An account with positions, balances and a history.
pub(crate) const USER: &str = "0x0123456789abcdef0123456789abcdef01234567";
/// An account that never traded.
pub(crate) const IDLE: &str = "0x00000000000000000000000000000000000000aa";
/// An account worth millions, whose figures are as wide as figures get.
pub(crate) const WHALE: &str = "0x1f250df59a777d61cb8bd043c12970f3afe4f925";
/// An account Hyperliquid takes [`SLOW_FOR`] to answer about, with
/// [`WHALE`]'s positions.
pub(crate) const SLOW: &str = "0x00000000000000000000000000000000000000cc";
/// How long [`SLOW`]'s replies take: less than a request's deadline.
pub(crate) const SLOW_FOR: Duration = Duration::from_secs(10);
/// An account whose requests Hyperliquid refuses, and Lighter.
pub(crate) const LIMITED: &str = "0x00000000000000000000000000000000000000ff";
/// An address with two accounts and a pool on Lighter.
pub(crate) const TRADER: &str = "0xe117d3d94b30f7e7aa4f29704d0d2f43c009b9a8";
/// The indexes of [`TRADER`]'s main account, its sub-account and its pool.
pub(crate) const TRADER_MAIN: u64 = 2_180;
pub(crate) const TRADER_SUB: u64 = 281_474_976_689_919;
const TRADER_POOL: u64 = 281_474_976_710_000;

/// What settings save for `address` on Hyperliquid.
pub(crate) fn account(address: &str) -> Value {
    json!({ "account": { "venue": "hyperliquid", "address": address } })
}

/// What settings save for `address` on Lighter.
pub(crate) fn lighter_account(address: &str) -> Value {
    json!({ "account": on("lighterxyz", address) })
}

/// What settings save for `address` on TxFlow.
pub(crate) fn txflow_account(address: &str) -> Value {
    json!({ "account": on("txflow", address) })
}

/// What settings save for `address` on `venue`, as one of several.
pub(crate) fn on(venue: &str, address: &str) -> Value {
    json!({ "venue": venue, "address": address })
}

/// What settings save for the several accounts `listed` ([`on`]).
pub(crate) fn accounts(listed: &[Value]) -> Value {
    json!({ "accounts": listed })
}

/// A workspace of one `kind` widget with `saved` as its settings (none:
/// its defaults), with its data services as the shell provides them, over
/// the stubbed Hyperliquid. What a test's root component returns.
pub(crate) fn workspace_of(kind: WidgetKind, saved: Option<Value>) -> impl IntoElement {
    let budget = HyperliquidConfig::production().weight_per_minute;
    workspace_within(kind, saved, budget)
}

/// [`workspace_of`] with `weight_per_minute` as all the app may ask of
/// Hyperliquid a minute.
pub(crate) fn workspace_within(
    kind: WidgetKind,
    saved: Option<Value>,
    weight_per_minute: u32,
) -> impl IntoElement {
    let services = DataServices::provide_with(DataConfig {
        ferris: FerrisConfig {
            http_base: "https://ferris.test".to_owned(),
            ws_url: "ws://127.0.0.1:9/v1/ws".to_owned(),
            ..FerrisConfig::production()
        },
        hyperliquid: HyperliquidConfig {
            http_base: HYPERLIQUID.to_owned(),
            weight_per_minute,
            ..HyperliquidConfig::production()
        },
        lighter: LighterConfig {
            http_base: LIGHTER.to_owned(),
            ..LighterConfig::production()
        },
        txflow: TxFlowConfig {
            http_base: TXFLOW.to_owned(),
            ..TxFlowConfig::production()
        },
        ..DataConfig::default()
    });
    provide_context(BarClock::restore());
    AddressGroups::provide();
    let mut workspace = StoredWorkspace::default();
    layout::normalize(&mut workspace);
    layout::spawn(&mut workspace, kind);
    if let Some(saved) = saved {
        let widget = layout::widget_ids(&workspace)
            .into_iter()
            .next()
            .expect("the widget just added");
        let settings = WidgetSettings::read(kind, saved);
        assert!(layout::set_tab_settings(&mut workspace, widget, &settings));
    }
    let workspace = signal(workspace);
    let picker = Picker::new();
    services
        .sessions
        .follow(move || layout::widget_ids(&workspace.get()));
    view()
        .size_full()
        .flex_col()
        .child(layout::workspace(workspace, picker))
        .child(picker.view(workspace))
}

/// The app rooted at `root` over the stubbed Hyperliquid, and the stub
/// network, which keeps the requests made.
pub(crate) fn start(root: Root) -> (Headless, Arc<StubNetwork>) {
    start_on(root, Clock::system(), MemoryStorage::new([]))
}

/// [`start`] on a clock that moves only when the test moves it
/// (`Headless::run_for`), for what happens over minutes.
pub(crate) fn start_manual(root: Root) -> (Headless, Arc<StubNetwork>) {
    start_on(
        root,
        Clock::manual(SystemTime::now()),
        MemoryStorage::new([]),
    )
}

/// [`start`] over storage that holds `saved` (keys and what is saved under
/// them), and that storage, which shows what the app saves.
pub(crate) fn start_storing(
    root: Root,
    saved: impl IntoIterator<Item = (String, String)>,
) -> (Headless, Arc<MemoryStorage>) {
    let storage = MemoryStorage::new(saved);
    let (app, _network) = start_on(root, Clock::system(), storage.clone());
    (app, storage)
}

fn start_on(root: Root, clock: Clock, storage: Arc<MemoryStorage>) -> (Headless, Arc<StubNetwork>) {
    // Delayed replies pass on the app's clock, so a test that moves it
    // moves them.
    let network = Arc::new(
        StubNetwork::new([
            (
                format!("POST {HYPERLIQUID}/info"),
                StubRoute::Handler(Arc::new(stub_hyperliquid)),
            ),
            (
                format!("GET {LIGHTER}/api/v1/*"),
                StubRoute::Handler(Arc::new(stub_lighter)),
            ),
            (
                format!("POST {TXFLOW}/info"),
                StubRoute::Handler(Arc::new(stub_txflow)),
            ),
        ])
        .with_clock(clock.clone()),
    );
    let mut config = EngineConfig::new(root);
    config.viewport = Size::new(px(1280.0), px(800.0));
    config.storage = Some(storage);
    config.network = Some(network.clone());
    config.clock = clock;
    config.environment.os = Os::Windows;
    config.environment.idiom = Idiom::Desktop;
    let app = Headless::start(config).expect("the app starts");
    (app, network)
}

/// The bodies of the info requests made of Hyperliquid so far, in order.
pub(crate) fn asked(network: &StubNetwork) -> Vec<Value> {
    asked_at(network, HYPERLIQUID)
}

/// The bodies of the info requests made of TxFlow so far, in order.
pub(crate) fn asked_txflow(network: &StubNetwork) -> Vec<Value> {
    asked_at(network, TXFLOW)
}

/// The bodies of the info requests made of the API at `origin` so far.
fn asked_at(network: &StubNetwork, origin: &str) -> Vec<Value> {
    let info = format!("{origin}/info");
    network
        .requests()
        .into_iter()
        .filter(|request| request.url == info)
        .filter_map(|request| match &request.body {
            HttpBody::Bytes(bytes) => serde_json::from_slice(bytes).ok(),
            _ => None,
        })
        .collect()
}

/// The paths and queries of the requests made of Lighter so far, in order
/// (`/api/v1/pnl?by=index&...`).
pub(crate) fn asked_lighter(network: &StubNetwork) -> Vec<String> {
    network
        .requests()
        .into_iter()
        .filter_map(|request| request.url.strip_prefix(LIGHTER).map(str::to_owned))
        .collect()
}

/// How many of `asked`'s requests were of `kind` for `user`.
pub(crate) fn count(asked: &[Value], kind: &str, user: &str) -> usize {
    asked
        .iter()
        .filter(|body| body["type"] == kind && body["user"] == user)
        .count()
}

/// [`USER`]'s perpetuals account: an isolated long and a larger cross
/// short.
pub(crate) fn clearinghouse_state() -> Value {
    json!({
        "assetPositions": [
            {
                "position": {
                    "coin": "ETH",
                    "cumFunding": { "allTime": "514.085417", "sinceChange": "0.0", "sinceOpen": "0.0" },
                    "entryPx": "2986.3",
                    "leverage": { "rawUsd": "-95.059824", "type": "isolated", "value": 20 },
                    "liquidationPx": "2866.26936529",
                    "marginUsed": "4.967826",
                    "maxLeverage": 50,
                    "positionValue": "100.02765",
                    "returnOnEquity": "-0.0026789",
                    "szi": "0.0335",
                    "unrealizedPnl": "-0.0134"
                },
                "type": "oneWay"
            },
            {
                "position": {
                    "coin": "BTC",
                    "cumFunding": { "allTime": "-40.0", "sinceChange": "0.0", "sinceOpen": "-12.5" },
                    "entryPx": "60000.0",
                    "leverage": { "type": "cross", "value": 5 },
                    "liquidationPx": null,
                    "marginUsed": "1220.0",
                    "maxLeverage": 40,
                    "positionValue": "6100.0",
                    "returnOnEquity": "0.0833",
                    "szi": "-0.1",
                    "unrealizedPnl": "100.0"
                },
                "type": "oneWay"
            }
        ],
        "crossMaintenanceMarginUsed": "61.0",
        "crossMarginSummary": {
            "accountValue": "13104.514502", "totalMarginUsed": "1220.0",
            "totalNtlPos": "6100.0", "totalRawUsd": "19204.514502"
        },
        "marginSummary": {
            "accountValue": "13109.482328", "totalMarginUsed": "1224.967826",
            "totalNtlPos": "6200.02765", "totalRawUsd": "19209.454678"
        },
        "time": 1_708_622_398_623_u64,
        "withdrawable": "11884.514502"
    })
}

/// [`WHALE`]'s perpetuals account: one cross position of $25.9M (as the
/// live account it is named after had).
fn whale_state() -> Value {
    let summary = json!({
        "accountValue": "2592775.550862", "totalMarginUsed": "648284.566749",
        "totalNtlPos": "25931382.66999", "totalRawUsd": "-23338607.119128"
    });
    json!({
        "assetPositions": [{
            "position": {
                "coin": "BTC", "entryPx": "64000.0",
                "leverage": { "type": "cross", "value": 40 },
                "liquidationPx": "58123.4", "marginUsed": "648284.566749",
                "positionValue": "25931382.66999", "returnOnEquity": "-0.1234",
                "szi": "-405.17", "unrealizedPnl": "-1234567.89",
                "cumFunding": { "allTime": "-99999.99", "sinceChange": "0.0", "sinceOpen": "-54321.0" }
            },
            "type": "oneWay"
        }],
        "crossMaintenanceMarginUsed": "324142.283374",
        "crossMarginSummary": summary,
        "marginSummary": summary,
        "time": 1_708_622_398_623_u64,
        "withdrawable": "0.0"
    })
}

/// An account with nothing in it.
fn empty_state() -> Value {
    json!({
        "assetPositions": [],
        "crossMaintenanceMarginUsed": "0.0",
        "marginSummary": {
            "accountValue": "0.0", "totalMarginUsed": "0.0", "totalNtlPos": "0.0",
            "totalRawUsd": "0.0"
        },
        "time": 1_708_622_398_623_u64,
        "withdrawable": "0.0"
    })
}

/// [`USER`]'s token balances.
pub(crate) fn spot_state() -> Value {
    json!({
        "balances": [
            { "coin": "USDC", "token": 0, "hold": "4.5", "total": "14.625485", "entryNtl": "0.0" },
            { "coin": "PURR", "token": 1, "hold": "0.0", "total": "2000.0", "entryNtl": "1234.56" },
            { "coin": "HYPE", "token": 150, "hold": "0.0", "total": "0.0", "entryNtl": "0.0" }
        ]
    })
}

/// One window of [`USER`]'s history: `points` samples an hour apart whose
/// value climbs from `from` by `step` each, the profit and loss counted
/// from the first.
fn window(points: u32, from: f64, step: f64) -> Value {
    const START: u64 = 1_741_886_630_493;
    const HOUR: u64 = 3_600_000;
    let at = |n: u32| START + u64::from(n) * HOUR;
    let value: Vec<Value> = (0..points)
        .map(|n| json!([at(n), format!("{:.2}", from + step * f64::from(n))]))
        .collect();
    let pnl: Vec<Value> = (0..points)
        .map(|n| json!([at(n), format!("{:.2}", step * f64::from(n))]))
        .collect();
    json!({ "accountValueHistory": value, "pnlHistory": pnl, "vlm": "250000.5" })
}

/// [`USER`]'s history: every window Hyperliquid reports, each with values
/// of its own so that a test can tell which one shows.
pub(crate) fn portfolio() -> Value {
    json!([
        ["day", window(30, 13_000.0, 2.0)],
        ["week", window(40, 12_000.0, 10.0)],
        ["month", window(50, 10_000.0, 50.0)],
        ["allTime", window(60, 1_000.0, 200.0)],
        ["perpDay", window(30, 9_000.0, -1.0)],
        ["perpWeek", window(40, 8_000.0, 5.0)],
        ["perpMonth", window(50, 7_000.0, 20.0)],
        ["perpAllTime", window(60, 500.0, 100.0)],
    ])
}

/// A history with nothing in it.
fn empty_portfolio() -> Value {
    let empty = json!({ "accountValueHistory": [], "pnlHistory": [], "vlm": "0.0" });
    let names = [
        "day",
        "week",
        "month",
        "allTime",
        "perpDay",
        "perpWeek",
        "perpMonth",
        "perpAllTime",
    ];
    Value::Array(names.map(|name| json!([name, empty])).to_vec())
}

/// The spot markets: PURR at 0.5 and HYPE at 40 USDC.
pub(crate) fn spot_meta() -> Value {
    json!([
        {
            "tokens": [
                { "name": "USDC", "index": 0 },
                { "name": "PURR", "index": 1 },
                { "name": "HYPE", "index": 150 }
            ],
            "universe": [
                { "tokens": [1, 0], "name": "PURR/USDC", "index": 0 },
                { "tokens": [150, 0], "name": "@107", "index": 107 }
            ]
        },
        [
            { "coin": "PURR/USDC", "markPx": "0.5", "midPx": "0.5" },
            { "coin": "@107", "markPx": "40.0", "midPx": "40.01" }
        ]
    ])
}

/// [`USER`]'s staking: 13 HYPE, 10 of it delegated.
fn delegator_summary() -> Value {
    json!({
        "delegated": "10.0", "undelegated": "2.0",
        "totalPendingWithdrawal": "1.0", "nPendingWithdrawals": 1
    })
}

/// [`USER`]'s fees: 500,000 traded a day for two weeks (tier 1), rates
/// with a 10% staking discount.
fn user_fees() -> Value {
    let days: Vec<Value> = (1..=15)
        .map(|day| {
            json!({
                "date": format!("2026-11-{day:02}"),
                "userCross": "400000.0", "userAdd": "100000.0", "exchange": "1000000000.0"
            })
        })
        .collect();
    json!({
        "dailyUserVlm": days,
        "feeSchedule": {
            "cross": "0.00045", "add": "0.00015",
            "tiers": { "vip": [
                { "ntlCutoff": "5000000.0", "cross": "0.0004", "add": "0.00012" },
                { "ntlCutoff": "25000000.0", "cross": "0.00035", "add": "0.00008" }
            ], "mm": [] }
        },
        "userCrossRate": "0.00036", "userAddRate": "0.000108",
        "userSpotCrossRate": "0.00054", "userSpotAddRate": "0.00027",
        "activeReferralDiscount": "0.0",
        "activeStakingDiscount": { "bpsOfMaxSupply": "0.001", "discount": "0.1" }
    })
}

/// [`USER`]'s trades, newest first as Hyperliquid writes them: an ETH
/// long opened, then part of the BTC short closed at a profit.
pub(crate) fn user_fills() -> Value {
    json!([
        { "coin": "BTC", "px": "59000.0", "sz": "0.05", "side": "B",
          "time": 1_708_622_398_623_u64, "startPosition": "-0.15", "dir": "Close Short",
          "closedPnl": "50.0", "hash": "0xb", "oid": 2, "crossed": false,
          "fee": "-0.295", "tid": 22, "feeToken": "USDC" },
        { "coin": "ETH", "px": "2986.3", "sz": "0.0335", "side": "B",
          "time": 1_708_622_000_000_u64, "startPosition": "0.0", "dir": "Open Long",
          "closedPnl": "0.0", "hash": "0xa", "oid": 1, "crossed": true,
          "fee": "0.045", "tid": 21, "feeToken": "USDC" }
    ])
}

/// Answers the stubbed Hyperliquid's info requests.
fn stub_hyperliquid(request: HttpRequest, reply: StubReply) {
    let body: Value = match &request.body {
        HttpBody::Bytes(bytes) => serde_json::from_slice(bytes).unwrap_or_default(),
        _ => Value::Null,
    };
    let user = body["user"].as_str().unwrap_or_default();
    let kind = body["type"].as_str().unwrap_or_default();
    let mut response = StubResponse::default();
    if user == LIMITED {
        response.status = 429;
        response.body = b"null".to_vec();
        reply.send(response);
        return;
    }
    if user == SLOW {
        response.delay = SLOW_FOR;
    }
    let known = user == USER;
    let answer = match kind {
        "clearinghouseState" if known => clearinghouse_state(),
        "clearinghouseState" if user == WHALE || user == SLOW => whale_state(),
        "clearinghouseState" => empty_state(),
        "spotClearinghouseState" if known => spot_state(),
        "spotClearinghouseState" => json!({ "balances": [] }),
        "portfolio" if known => portfolio(),
        "portfolio" => empty_portfolio(),
        "spotMetaAndAssetCtxs" => spot_meta(),
        "delegatorSummary" if known => delegator_summary(),
        "delegatorSummary" => json!({
            "delegated": "0.0", "undelegated": "0.0",
            "totalPendingWithdrawal": "0.0", "nPendingWithdrawals": 0
        }),
        "userVaultEquities" if known => json!([
            { "vaultAddress": "0xdfc24b077bc1425ad1dea75bcb6f8158e10df303",
              "equity": "1000.0", "lockedUntilTimestamp": 1_749_486_275_752_u64 }
        ]),
        "userVaultEquities" => json!([]),
        "userFills" if known => user_fills(),
        "userFills" => json!([]),
        "userFees" if known => user_fees(),
        "userFees" => json!({
            "dailyUserVlm": [],
            "userCrossRate": "0.00045", "userAddRate": "0.00015",
            "userSpotCrossRate": "0.0007", "userSpotAddRate": "0.0004"
        }),
        _ => {
            response.status = 422;
            response.body = b"Failed to deserialize the JSON body into the target type".to_vec();
            reply.send(response);
            return;
        }
    };
    response.status = 200;
    response.body = serde_json::to_vec(&answer).expect("the answer is JSON");
    reply.send(response);
}

/// One of the stubbed Lighter's accounts, as its samples say it.
struct Sampled {
    /// What was moved into its perpetuals, and into its spot.
    put_in: f64,
    spot: f64,
    /// What its perpetuals have made by now.
    made: f64,
    /// What they made between two samples (below zero: lost).
    gained: f64,
    /// What it traded between two samples.
    traded: f64,
    /// How many samples ago it began: nothing is sampled before.
    began: u64,
}

impl Sampled {
    /// Its samples between `from` and `to` (the times a request names, in
    /// seconds): one on every `step` seconds and one more at `to`, as
    /// Lighter writes them.
    fn between(&self, (from, to): (u64, u64), step: u64) -> Value {
        let sample = |at: u64| {
            let ago = (to - at).div_ceil(step);
            (ago <= self.began).then(|| {
                json!({
                    "timestamp": at,
                    "trade_pnl": self.made - self.gained * ago as f64,
                    "inflow": self.put_in,
                    "spot_inflow": self.spot,
                    "volume": self.traded,
                })
            })
        };
        let first = from.div_ceil(step) * step;
        let samples: Vec<Value> = (first..to)
            .step_by(step as usize)
            .chain([to])
            .filter_map(sample)
            .collect();
        json!({ "code": 200, "pnl": samples })
    }
}

/// [`TRADER`]'s history of the account `index`, as asked for: its main
/// account is worth 21,500 now (500 of it in spot) and made 5 an hour and
/// 20 a day for a hundred days, trading 1,000 each; its sub-account is
/// worth 2,900 and lost 2 an hour and 10 a day since it began ten days
/// ago, trading 100 each. `None` for any other account.
fn lighter_pnl(index: u64, resolution: &str, span: (u64, u64)) -> Option<Value> {
    let (step, hourly) = match resolution {
        "1h" => (3_600, true),
        "1d" => (86_400, false),
        _ => return None,
    };
    let by = |an_hour: f64, a_day: f64| if hourly { an_hour } else { a_day };
    let lasted = |days: u64| if hourly { days * 24 } else { days };
    let account = match index {
        TRADER_MAIN => Sampled {
            put_in: 20_000.0,
            spot: 500.0,
            made: 1_000.0,
            gained: by(5.0, 20.0),
            traded: 1_000.0,
            began: lasted(100),
        },
        TRADER_SUB => Sampled {
            put_in: 3_000.0,
            spot: 0.0,
            made: -100.0,
            gained: by(-2.0, -10.0),
            traded: 100.0,
            began: lasted(10),
        },
        _ => return None,
    };
    Some(account.between(span, step))
}

/// [`TRADER`]'s accounts, as Lighter lists them. Its main account is worth
/// 21,000 with a BTC long and an isolated SOL short, and holds LIT and
/// USDC besides; its sub-account is worth 2,900 with a BTC short; the pool
/// it runs is not its own.
fn lighter_accounts() -> Value {
    json!({
        "code": 200,
        "total": 3,
        "accounts": [
            {
                "code": 0, "account_type": 0, "index": TRADER_MAIN, "status": 1,
                "available_balance": "19000.000000", "collateral": "19500.000000",
                "total_asset_value": "21000.000000", "cross_asset_value": "21000.000000",
                "cross_initial_margin_requirement": "1550.000000",
                "cross_maintenance_margin_requirement": "900.000000",
                "positions": [
                    { "market_id": 1, "symbol": "BTC", "initial_margin_fraction": "5.00",
                      "sign": 1, "position": "0.50000", "avg_entry_price": "60000.0",
                      "position_value": "31000.000000", "unrealized_pnl": "1000.000000",
                      "liquidation_price": "30000.5", "total_funding_paid_out": "-12.500000",
                      "margin_mode": 0, "allocated_margin": "0.000000" },
                    { "market_id": 14, "symbol": "POL", "initial_margin_fraction": "33.33",
                      "sign": 1, "position": "0", "avg_entry_price": "0.000000",
                      "position_value": "-0.000000", "unrealized_pnl": "0.000000",
                      "liquidation_price": "0", "margin_mode": 0,
                      "allocated_margin": "0.000000" },
                    { "market_id": 2, "symbol": "SOL", "initial_margin_fraction": "10.00",
                      "sign": -1, "position": "100.000", "avg_entry_price": "70.000",
                      "position_value": "6500.000000", "unrealized_pnl": "500.000000",
                      "liquidation_price": "120.25", "total_funding_paid_out": "2.500000",
                      "margin_mode": 1, "allocated_margin": "650.000000" },
                ],
                "assets": [
                    { "symbol": "LIT", "asset_id": 2, "balance": "100.00000000",
                      "locked_balance": "10.00000000", "margin_mode": "disabled",
                      "margin_balance": "0.00000000" },
                    { "symbol": "USDC", "asset_id": 3, "balance": "250.000000",
                      "locked_balance": "0.000000", "margin_mode": "enabled",
                      "margin_balance": "19500.000000" },
                    { "symbol": "ETH", "asset_id": 1, "balance": "0.00000000",
                      "locked_balance": "0.00000000" },
                ],
                "shares": null,
            },
            {
                "code": 0, "account_type": 1, "index": TRADER_SUB, "status": 0,
                "available_balance": "2700.000000", "collateral": "2910.000000",
                "total_asset_value": "2900.000000", "cross_asset_value": "2900.000000",
                "cross_initial_margin_requirement": "31.000000",
                "cross_maintenance_margin_requirement": "60.000000",
                "positions": [
                    { "market_id": 1, "symbol": "BTC", "initial_margin_fraction": "5.00",
                      "sign": -1, "position": "0.01000", "avg_entry_price": "61000.0",
                      "position_value": "620.000000", "unrealized_pnl": "-10.000000",
                      "liquidation_price": "300000.0", "total_funding_paid_out": "0.250000",
                      "margin_mode": 0, "allocated_margin": "0.000000" },
                ],
                "assets": [],
            },
            {
                "code": 0, "account_type": 3, "index": TRADER_POOL, "status": 1,
                "total_asset_value": "79030542.824306",
                "positions": [
                    { "symbol": "BTC", "sign": 1, "position": "9.0",
                      "position_value": "560000.0" },
                ],
            },
        ],
    })
}

/// When [`TRADER`]'s latest trade was, in Unix milliseconds; the others
/// were a minute apart before it.
const TRADED: u64 = 1_741_900_000_000;

/// The latest trades of the account `index` of [`TRADER`]'s, newest first:
/// its main account bought BTC taking, sold ETH making (closing a long),
/// and sold BTC to its own sub-account, whose only trade that is.
fn lighter_trades(index: u64) -> Value {
    let between = json!({
        "trade_id": 9_001, "type": "trade", "market_id": 1, "market_symbol": "BTC",
        "size": "0.01000", "price": "61000.0", "usd_amount": "610.000000",
        "ask_account_id": TRADER_MAIN, "bid_account_id": TRADER_SUB,
        "is_maker_ask": false, "timestamp": TRADED - 120_000,
    });
    let trades = match index {
        TRADER_MAIN => vec![
            json!({
                "trade_id": 9_003, "type": "trade", "market_id": 1, "market_symbol": "BTC",
                "size": "0.50000", "price": "60000.0", "usd_amount": "30000.000000",
                "ask_account_id": 77, "bid_account_id": TRADER_MAIN,
                "is_maker_ask": true, "timestamp": TRADED, "taker_fee": 280, "maker_fee": 20,
            }),
            json!({
                "trade_id": 9_002, "type": "trade", "market_id": 0, "market_symbol": "ETH",
                "size": "1.0000", "price": "1800.00", "usd_amount": "1800.000000",
                "ask_account_id": TRADER_MAIN, "bid_account_id": 78,
                "is_maker_ask": true, "timestamp": TRADED - 60_000, "taker_fee": 280,
                "maker_fee": 20, "ask_account_pnl": "25.500000",
            }),
            between,
        ],
        TRADER_SUB => vec![between],
        _ => Vec::new(),
    };
    json!({ "code": 200, "trades": trades })
}

/// What the stubbed Lighter's assets are worth.
fn lighter_assets() -> Value {
    json!({
        "code": 200,
        "asset_details": [
            { "asset_id": 3, "symbol": "USDC", "index_price": "1.000000" },
            { "asset_id": 2, "symbol": "LIT", "index_price": "2.500000" },
            { "asset_id": 1, "symbol": "ETH", "index_price": "1800.000000" },
        ],
    })
}

/// Answers the stubbed Lighter's requests.
fn stub_lighter(request: HttpRequest, reply: StubReply) {
    let mut response = StubResponse::default();
    let Ok(url) = url::Url::parse(&request.url) else {
        response.status = 400;
        reply.send(response);
        return;
    };
    let asked = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
            .unwrap_or_default()
    };
    // The times are asked for in milliseconds.
    let seconds = |name: &str| asked(name).parse::<u64>().unwrap_or_default() / 1000;
    let answer = match url.path() {
        "/api/v1/account" => match asked("value").as_str() {
            LIMITED => {
                response.status = 429;
                reply.send(response);
                return;
            }
            TRADER => Some(lighter_accounts()),
            _ => None,
        },
        "/api/v1/trades" => Some(lighter_trades(
            asked("account_index").parse().unwrap_or_default(),
        )),
        "/api/v1/assetDetails" => Some(lighter_assets()),
        "/api/v1/pnl" => lighter_pnl(
            asked("value").parse().unwrap_or_default(),
            &asked("resolution"),
            (seconds("start_timestamp"), seconds("end_timestamp")),
        ),
        _ => None,
    };
    let answer = answer.unwrap_or_else(|| {
        response.status = 400;
        json!({ "code": 21100, "message": "account not found" })
    });
    if response.status != 400 {
        response.status = 200;
    }
    response.body = serde_json::to_vec(&answer).expect("the answer is JSON");
    reply.send(response);
}

/// [`USER`]'s perpetuals sub-account on TxFlow: an isolated BTC long.
fn txflow_state() -> Value {
    json!({
        "assetPositions": [{
            "position": {
                "coin": "BTC-USDC", "asset": "1", "entryPx": "60000.0",
                "leverage": { "type": "Isolated", "value": 10, "rawUsd": "-27900.0" },
                "liquidationPx": "54321.0", "marginUsed": "3100.0",
                "positionValue": "31000.0", "returnOnEquity": "32.258064",
                "szi": "0.5", "unrealizedPnl": "1000.0", "maxLeverage": 75,
                "cumFunding": { "allTime": "", "sinceOpen": "4.5", "sinceChange": "4.5" },
                "markPx": "62000.0", "updateTime": 1_710_364_000_000_u64
            },
            "type": "OneWay"
        }],
        "crossMarginSummary": {
            "accountValue": "16900.0", "totalMarginUsed": "0.0",
            "totalNtlPos": "0.0", "totalRawUsd": "16900.0"
        },
        "marginSummary": {
            "accountValue": "20000.0", "totalMarginUsed": "3100.0",
            "totalNtlPos": "31000.0", "totalRawUsd": "-11000.0"
        },
        "crossMaintenanceMarginUsed": "0.0", "crossMarginRatio": "0.0000",
        "withdrawable": "16900.0", "vaultEquity": "0.0",
        "time": 1_710_364_000_000_u64
    })
}

/// [`USER`]'s latest trades on TxFlow: the BTC long opened, and an ETH
/// long closed.
fn txflow_fills() -> Value {
    json!({
        "data": [
            { "coin": "BTC", "symbol": "BTC-USDC", "px": "60000.0", "sz": "0.5", "side": "B",
              "time": 1_710_364_000_000_u64, "startPosition": "0", "dir": "Buy",
              "closedPnl": "0", "hash": "0x01", "oid": 1, "crossed": null, "fee": "8.4",
              "tid": 101, "feeToken": "USDC", "tradeSide": "Taker", "instrumentId": 1 },
            { "coin": "ETH", "symbol": "ETH-USDC", "px": "1800.0", "sz": "1", "side": "A",
              "time": 1_710_363_880_000_u64, "startPosition": "2", "dir": "Sell",
              "closedPnl": "25.5", "hash": "0x02", "oid": 2, "crossed": null, "fee": "0.036",
              "tid": 102, "feeToken": "USDC", "tradeSide": "Maker", "instrumentId": 2 }
        ],
        "hasMore": false,
        "total": 2
    })
}

/// What [`USER`] made over `period` on TxFlow (`24H`, `7D`, `30D`), and
/// what it traded.
fn txflow_overview(period: &str, known: bool) -> Value {
    let (pnl, volume) = match period {
        _ if !known => ("0", "0"),
        "24H" => ("72.0", "26400.0"),
        "7D" => ("504.0", "90000.0"),
        _ => ("500.0", "250000.0"),
    };
    json!({
        "code": 0, "msg": "", "period": period, "pnl": pnl, "roe": "0.36",
        "volume": volume, "maxDrawdown": "1.2", "totalEquity": "20000.0"
    })
}

/// TxFlow's fee schedule, with `daily` as the user's volume of yesterday.
fn txflow_fees(daily: &str) -> Value {
    let yesterday = (unix_ms() / 86_400_000.0).floor() * 86_400_000.0 - 86_400_000.0;
    json!({
        "activeReferralDiscount": "0.1",
        "dailyUserVlm": [{
            "date": format!("{yesterday:.0}"), "exchange": daily, "userAdd": daily, "userCross": daily
        }],
        "feeSchedule": { "tiers": { "mm": [], "vip": [
            { "add": "0.000150", "cross": "0.000450", "ntlCutoff": "0.00" },
            { "add": "0.000120", "cross": "0.000400", "ntlCutoff": "5000000.00" }
        ] } },
        "userAddRate": "0.000135", "userCrossRate": "0.000405"
    })
}

/// Answers the stubbed TxFlow's info requests, wrapped as TxFlow wraps
/// them (but for the history's). It knows [`USER`]; any other address is
/// an empty account, and [`LIMITED`]'s requests are refused.
fn stub_txflow(request: HttpRequest, reply: StubReply) {
    let body: Value = match &request.body {
        HttpBody::Bytes(bytes) => serde_json::from_slice(bytes).unwrap_or_default(),
        _ => Value::Null,
    };
    let user = body["user"]
        .as_str()
        .or_else(|| body["user_address"].as_str())
        .unwrap_or_default();
    let known = user == USER;
    let mut response = StubResponse::default();
    if user == LIMITED {
        response.status = 429;
        response.body =
            br#"{"success":false,"code":10000,"message":"rate limit exceeded"}"#.to_vec();
        reply.send(response);
        return;
    }
    let data = match body["type"].as_str().unwrap_or_default() {
        "clearinghouseState" if known => txflow_state(),
        "clearinghouseState" => json!({
            "assetPositions": [],
            "marginSummary": { "accountValue": "0.000000", "totalMarginUsed": "0.000000",
                               "totalNtlPos": "0.000000", "totalRawUsd": "0.000000" },
            "crossMaintenanceMarginUsed": "0.000000", "withdrawable": "0.000000",
            "time": 1_710_364_000_000_u64
        }),
        "userFills" if known => txflow_fills(),
        "userFills" => json!({ "data": [], "hasMore": false, "total": 0 }),
        "userFees" => txflow_fees(if known { "6000000.0" } else { "0" }),
        "uservaultequities" if known => json!([
            { "vaultAddress": "0x664989a76069c2b153720745bc1c3c8eeee504c1",
              "equity": "1000.0", "lockedUntilTimestamp": 1_792_830_521_530_u64 }
        ]),
        "uservaultequities" => json!([]),
        "userOverview" => {
            let period = body["period"].as_str().unwrap_or_default();
            response.status = 200;
            response.body =
                serde_json::to_vec(&txflow_overview(period, known)).expect("the answer is JSON");
            reply.send(response);
            return;
        }
        _ => {
            response.status = 400;
            response.body =
                br#"{"success":false,"code":10000,"message":"This action is not allowed."}"#
                    .to_vec();
            reply.send(response);
            return;
        }
    };
    response.status = 200;
    response.body = serde_json::to_vec(&json!({ "success": true, "code": 0, "data": data }))
        .expect("the answer is JSON");
    reply.send(response);
}
