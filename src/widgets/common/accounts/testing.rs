//! What the account widgets' tests share: a workspace of one widget over
//! data services whose Hyperliquid is the test's stub network, and that
//! stub's replies.
//!
//! The stubbed Hyperliquid knows three addresses: [`USER`], an account with
//! two positions, three tokens, a history, staked HYPE, a vault share and
//! a fee tier; [`IDLE`], one that never traded; and [`LIMITED`], whose
//! every request is refused with a `429`. The tokens' prices are the same
//! for every account.

use std::{sync::Arc, time::SystemTime};

use nsh_core::{
    element::Root,
    platform::{Idiom, Os},
    prelude::*,
    time::Clock,
};
use nsh_engine::{EngineConfig, HttpBody, HttpRequest};
use nsh_testing::{Headless, MemoryStorage, StubNetwork, StubReply, StubResponse, StubRoute};
use serde_json::{json, Value};

use crate::{
    data::{ferris::FerrisConfig, hyperliquid::HyperliquidConfig, DataConfig, DataServices},
    layout,
    shell::clock::Clock as BarClock,
    storage::StoredWorkspace,
    widgets::{picker::Picker, WidgetKind, WidgetSettings},
};

/// A Hyperliquid no test reaches: its HTTP is the test's stub network.
pub(crate) const HYPERLIQUID: &str = "https://hyperliquid.test";
/// An account with positions, balances and a history.
pub(crate) const USER: &str = "0x0123456789abcdef0123456789abcdef01234567";
/// An account that never traded.
pub(crate) const IDLE: &str = "0x00000000000000000000000000000000000000aa";
/// An account worth millions, whose figures are as wide as figures get.
pub(crate) const WHALE: &str = "0x1f250df59a777d61cb8bd043c12970f3afe4f925";
/// An account whose requests Hyperliquid refuses.
pub(crate) const LIMITED: &str = "0x00000000000000000000000000000000000000ff";

/// What settings save for `address` on Hyperliquid.
pub(crate) fn account(address: &str) -> Value {
    json!({ "account": { "venue": "hyperliquid", "address": address } })
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
    });
    provide_context(BarClock::restore());
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
    start_on(root, Clock::system())
}

/// [`start`] on a clock that moves only when the test moves it
/// (`Headless::run_for`), for what happens over minutes.
pub(crate) fn start_manual(root: Root) -> (Headless, Arc<StubNetwork>) {
    start_on(root, Clock::manual(SystemTime::now()))
}

fn start_on(root: Root, clock: Clock) -> (Headless, Arc<StubNetwork>) {
    let network = Arc::new(StubNetwork::new([(
        format!("POST {HYPERLIQUID}/info"),
        StubRoute::Handler(Arc::new(stub_hyperliquid)),
    )]));
    let mut config = EngineConfig::new(root);
    config.viewport = Size::new(px(1280.0), px(800.0));
    config.storage = Some(MemoryStorage::new([]));
    config.network = Some(network.clone());
    config.clock = clock;
    config.environment.os = Os::Windows;
    config.environment.idiom = Idiom::Desktop;
    let app = Headless::start(config).expect("the app starts");
    (app, network)
}

/// The bodies of the info requests made so far, in order.
pub(crate) fn asked(network: &StubNetwork) -> Vec<Value> {
    network
        .requests()
        .into_iter()
        .filter(|request| request.url.ends_with("/info"))
        .filter_map(|request| match &request.body {
            HttpBody::Bytes(bytes) => serde_json::from_slice(bytes).ok(),
            _ => None,
        })
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
    let known = user == USER;
    let answer = match kind {
        "clearinghouseState" if known => clearinghouse_state(),
        "clearinghouseState" if user == WHALE => whale_state(),
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
