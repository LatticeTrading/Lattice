//! What the tests of the widgets that show DefiLlama's data share: a
//! stubbed Ferris that answers its DefiLlama routes as the live one does
//! (shapes read off it on 2026-11-21), and a workspace of one widget over
//! it.

use std::sync::Arc;

use nsh_core::{
    element::Root,
    platform::{Idiom, Os},
    prelude::*,
};
use nsh_engine::{EngineConfig, HttpRequest};
use nsh_testing::{Headless, MemoryStorage, StubNetwork, StubReply, StubResponse, StubRoute};
use serde_json::{Value, json};

use crate::{
    data::{DataServices, ferris::FerrisConfig},
    layout,
    shell::clock::Clock,
    storage::StoredWorkspace,
    widgets::{WidgetKind, WidgetSettings, picker::Picker},
};

/// A Ferris no test reaches: its HTTP is the test's stub network, and
/// nothing listens where its socket would connect.
const FERRIS: &str = "https://ferris.test";
/// A day, in milliseconds.
const DAY_MS: u64 = 86_400_000;
/// The start of the stubbed Hyperliquid's latest day, and its latest
/// reading (later that day, as the live history's last point is).
const LATEST_DAY_MS: u64 = 1_795_219_200_000;
pub(crate) const LATEST_MS: u64 = 1_795_268_687_000;
/// The days of history the stubbed Hyperliquid has, its TVL `days` days
/// before its latest day ([`day_tvl`]), and its latest reading.
pub(crate) const DAYS: u64 = 400;
pub(crate) const LATEST_TVL: f64 = 7_151_116_329.0;

/// The stubbed Hyperliquid's TVL at the start of the day `ago` days
/// before its latest.
pub(crate) fn day_tvl(ago: u64) -> f64 {
    7_000_000_000.0 - ago as f64 * 10_000_000.0
}

/// A row of the stubbed list.
fn row(slug: &str, name: &str, symbol: Value, category: &str, tvl: Value, parent: Value) -> Value {
    json!({
        "slug": slug, "name": name, "symbol": symbol, "category": category,
        "chains": match category {
            "CEX" => json!(["Ethereum", "Bitcoin", "Solana"]),
            "Derivatives" => json!(["Solana"]),
            _ => json!(["Ethereum", "Arbitrum"]),
        },
        "logo": format!("https://icons.llamao.fi/icons/protocols/{slug}"),
        "url": "https://example.com",
        "tvl": tvl,
        "change1h": 0.25, "change1d": -1.5, "change7d": if slug == "lido" { json!(null) } else { json!(4.0) },
        "mcap": if slug == "lido" { json!(1_000_000_000.0) } else { json!(null) },
        "parentSlug": parent,
        "listedAt": 1_668_170_565_000_u64,
    })
}

/// The stubbed list, largest TVL first, the ones with none last.
fn listed() -> Vec<Value> {
    vec![
        row(
            "binance-cex",
            "Binance CEX",
            json!("BNB"),
            "CEX",
            json!(108_117_746_518.9),
            Value::Null,
        ),
        row(
            "lido",
            "Lido",
            json!("LDO"),
            "Liquid Staking",
            json!(20_000_000_000.0),
            Value::Null,
        ),
        row(
            "aave-v3",
            "Aave V3",
            json!("AAVE"),
            "Lending",
            json!(15_500_000_000.0),
            json!("aave"),
        ),
        row(
            "hyperliquid-bridge",
            "Hyperliquid Bridge",
            json!("HYPE"),
            "Bridge",
            json!(5_646_770_916.9),
            json!("hyperliquid"),
        ),
        row(
            "jupiter-perpetual-exchange",
            "Jupiter Perpetual Exchange",
            json!("JUP"),
            "Derivatives",
            json!(567_102_609.6),
            json!("jupiter"),
        ),
        row(
            "trade[xyz]",
            "trade[XYZ]",
            Value::Null,
            "Derivatives",
            json!(950.0),
            Value::Null,
        ),
        row(
            "hyperliquid-perps",
            "Hyperliquid Perps",
            json!("HYPE"),
            "Derivatives",
            Value::Null,
            json!("hyperliquid"),
        ),
    ]
}

/// What the stubbed Ferris lists for `query` (`category`, `search` and
/// `limit`, as the live route reads them). Unfiltered, it says DefiLlama
/// lists far more than it sends.
fn list_reply(query: &[(String, String)]) -> Value {
    let of = |name: &str| {
        query
            .iter()
            .find(|(member, _)| member == name)
            .map(|(_, value)| value.as_str())
    };
    let mut rows = listed();
    if let Some(category) = of("category") {
        rows.retain(|row| row["category"] == category);
    }
    if let Some(search) = of("search").map(str::to_lowercase) {
        rows.retain(|row| {
            ["slug", "name", "symbol"].iter().any(|member| {
                row[member]
                    .as_str()
                    .is_some_and(|text| text.to_lowercase().contains(&search))
            })
        });
    }
    let total = match (of("category"), of("search")) {
        (None, None) => 9_086,
        _ => rows.len(),
    };
    let limit = of("limit")
        .and_then(|limit| limit.parse().ok())
        .unwrap_or(500);
    rows.truncate(limit);
    json!({
        "source": "defillama", "sourceUrl": "https://defillama.com",
        "fetchedAt": LATEST_MS, "stale": false, "total": total, "protocols": rows,
    })
}

/// What the stubbed Ferris says of the protocol `slug`; `None` for one
/// DefiLlama does not list.
fn protocol_reply(slug: &str) -> Option<Value> {
    let (name, category, parent, tvl, chains, extras, history) = match slug {
        "hyperliquid" => {
            let mut history: Vec<Value> = (0..DAYS)
                .rev()
                .map(|ago| json!([LATEST_DAY_MS - ago * DAY_MS, day_tvl(ago)]))
                .collect();
            history.push(json!([LATEST_MS, LATEST_TVL]));
            (
                "Hyperliquid",
                Value::Null,
                true,
                json!(LATEST_TVL),
                json!({ "Arbitrum": 76_910_273.0, "Hyperliquid L1": 7_074_206_055.0 }),
                json!({ "borrowed": 355_967_352.0 }),
                history,
            )
        }
        "binance-cex" => (
            "Binance CEX",
            json!("CEX"),
            false,
            json!(109_473_098_813.0),
            json!({ "Ethereum": 60_000_000_000.0, "Bitcoin": 49_473_098_813.0 }),
            json!({ "OwnTokens": 19_995_827_123.6 }),
            vec![
                json!([LATEST_DAY_MS - DAY_MS, 120_000_000_000.0]),
                json!([LATEST_DAY_MS, 110_000_000_000.0]),
                json!([LATEST_MS, 109_473_098_813.0]),
            ],
        ),
        "variational" => (
            "Variational",
            json!("Derivatives"),
            false,
            Value::Null,
            json!({}),
            json!({}),
            Vec::new(),
        ),
        _ => return None,
    };
    Some(json!({
        "source": "defillama",
        "sourceUrl": format!("https://defillama.com/protocol/{slug}"),
        "fetchedAt": LATEST_MS,
        "stale": false,
        "protocol": {
            "slug": slug, "name": name, "symbol": null, "category": category,
            "chains": [], "description": "A stubbed protocol.", "parentSlug": null,
            "isParent": parent, "mcap": null,
        },
        "tvl": tvl,
        "chainTvls": chains,
        "extraTvls": extras,
        "tvlHistory": history,
    }))
}

/// Answers the stubbed Ferris's DefiLlama routes.
fn stub(request: HttpRequest, reply: StubReply) {
    let url = url::Url::parse(&request.url).ok();
    let path = url
        .as_ref()
        .map(|url| url.path().to_owned())
        .unwrap_or_default();
    let query: Vec<(String, String)> = url
        .as_ref()
        .map(|url| url.query_pairs().into_owned().collect())
        .unwrap_or_default();
    let (status, answer) = match path.strip_prefix("/v1/defillama/protocol/") {
        _ if path == "/v1/defillama/protocols" => (200, list_reply(&query)),
        Some(slug) => match protocol_reply(slug) {
            Some(answer) => (200, answer),
            None => (
                404,
                json!({
                    "code": "PROTOCOL_NOT_FOUND",
                    "message": format!("protocol `{slug}` is not listed on DefiLlama"),
                }),
            ),
        },
        None => (
            404,
            json!({ "code": "NOT_FOUND", "message": "no such route" }),
        ),
    };
    let mut response = StubResponse::default();
    response.status = status;
    response.body = serde_json::to_vec(&answer).expect("the answer is JSON");
    reply.send(response);
}

/// A workspace of one `kind` widget with `saved` as its settings (none:
/// its defaults), with its data services as the shell provides them, over
/// the stubbed Ferris. What a test's root component returns.
pub(crate) fn workspace_of(kind: WidgetKind, saved: Option<Value>) -> impl IntoElement {
    let services = DataServices::provide_with(FerrisConfig {
        http_base: FERRIS.to_owned(),
        ws_url: "ws://127.0.0.1:9/v1/ws".to_owned(),
        ..FerrisConfig::production()
    });
    provide_context(Clock::restore());
    let mut workspace = StoredWorkspace::default();
    layout::normalize(&mut workspace);
    layout::spawn(&mut workspace, kind);
    let widget = layout::widget_ids(&workspace)
        .into_iter()
        .next()
        .expect("the widget just added");
    if let Some(saved) = saved {
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

/// The app rooted at `root` over the stubbed Ferris, and the stub network,
/// which keeps the requests made.
pub(crate) fn start(root: Root) -> (Headless, Arc<StubNetwork>) {
    let network = Arc::new(StubNetwork::new([(
        format!("GET {FERRIS}/v1/defillama/*"),
        StubRoute::Handler(Arc::new(stub)),
    )]));
    let mut config = EngineConfig::new(root);
    config.viewport = Size::new(px(1280.0), px(800.0));
    config.storage = Some(MemoryStorage::new([]));
    config.network = Some(network.clone());
    config.environment.os = Os::Windows;
    config.environment.idiom = Idiom::Desktop;
    let app = Headless::start(config).expect("the app starts");
    (app, network)
}

/// What was asked of the stubbed Ferris's DefiLlama routes so far: each
/// request's path and query, in order.
pub(crate) fn asked(network: &StubNetwork) -> Vec<String> {
    network
        .requests()
        .into_iter()
        .filter_map(|request| {
            request
                .url
                .strip_prefix(FERRIS)
                .filter(|path| path.starts_with("/v1/defillama/"))
                .map(str::to_owned)
        })
        .collect()
}
