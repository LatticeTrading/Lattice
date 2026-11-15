use std::{
    rc::Rc,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use nsh_core::{
    element::Root,
    platform::{Idiom, Os},
    prelude::*,
};
use nsh_engine::{EngineConfig, HttpBody, HttpRequest};
use nsh_testing::{Headless, MemoryStorage, StubNetwork, StubReply, StubResponse, StubRoute, WAIT};
use serde_json::{Value, json};

use super::{
    session::{self, Row, Sign},
    settings::{FundingRatesSettings, PairKey, Period, Refused, Toggle},
};
use crate::{
    data::{
        DataServices,
        ferris::{
            Capabilities, FerrisConfig, MarketSelection, Product, StatsFieldName, StatsSnapshot,
            stats_scopes, universe::FeedSnap,
        },
        net::ApiError,
    },
    layout,
    shell::clock::Clock,
    storage::StoredWorkspace,
    widgets::{WidgetKind, WidgetSettings, picker::Picker},
};

/// A catalog's row for a market, and the id its statistics share.
fn catalog_row(exchange: &str, base: &str, quote: &str, kind: &str) -> Value {
    let perp = kind == "perp";
    json!({
        "exchange": exchange,
        "symbol": format!("{base}/{quote}"),
        "base": base,
        "quote": quote,
        "type": kind,
        "active": true,
        "marketId": format!("{exchange}:{kind}:{base}{quote}"),
        "exchangeMarketId": format!("{base}{quote}"),
        "category": if exchange == "bybit" && perp { json!("linear") } else { Value::Null },
        "settle": if perp { json!(quote) } else { Value::Null },
        "info": { "exchangeSymbol": format!("{base}{quote}") },
    })
}

/// A perpetual as the market picker hands it over.
fn perp(exchange: &str, base: &str, quote: &str) -> MarketSelection {
    MarketSelection::from_catalog(&catalog_row(exchange, base, quote, "perp"), exchange)
        .expect("a catalog row with a symbol")
}

fn saved(markets: &[MarketSelection]) -> Value {
    json!({ "markets": markets })
}

#[test]
fn settings_save_only_what_differs_and_read_leniently() {
    // Defaults save nothing, whatever the old app wrote out for them.
    let defaults = FundingRatesSettings::default();
    assert_eq!(serde_json::to_value(&defaults).unwrap(), json!({}));
    let written = json!({
        "showExchange": true, "showFundingRate": true, "showHourlyRate": true,
        "showEightHourRate": false, "showDailyRate": false, "showAnnualizedRate": true,
        "showPaymentInterval": true, "showAge": true,
    });
    assert_eq!(FundingRatesSettings::from(written), defaults);
    assert_eq!(defaults.periods(), [Period::Hour, Period::Year]);

    // The switches, under the old names; one of the wrong type is its default.
    let settings = FundingRatesSettings::from(json!({
        "showEightHourRate": true, "showAge": false, "showExchange": "no",
    }));
    assert!(settings.shows(Toggle::EightHour) && !settings.shows(Toggle::Age));
    assert!(settings.shows(Toggle::Exchange));
    assert_eq!(
        serde_json::to_value(&settings).unwrap(),
        json!({ "showEightHourRate": true, "showAge": false })
    );
    assert_eq!(
        settings.periods(),
        [Period::Hour, Period::EightHours, Period::Year]
    );
    // No rate column shows while the rates are off.
    assert!(
        settings
            .with_toggle(Toggle::Rates, false)
            .periods()
            .is_empty()
    );

    // Markets: perpetuals with a catalog id, each once; the rest are
    // counted, and the count is not saved.
    let (btc, eth) = (perp("bybit", "BTC", "USDT"), perp("bybit", "ETH", "USDT"));
    let mut spot = btc.clone();
    spot.kind = crate::data::ferris::MarketType::Spot;
    let mut legacy = eth.clone();
    legacy.market_id = None;
    let settings = FundingRatesSettings::from(json!({
        "markets": [btc, spot, eth, legacy, btc, null, "BTC"],
    }));
    assert_eq!(settings.markets(), [btc.clone(), eth.clone()]);
    assert_eq!(settings.rejected(), 4);
    assert_eq!(
        serde_json::to_value(&settings).unwrap(),
        saved(&[btc.clone(), eth.clone()])
    );
    assert_eq!(
        FundingRatesSettings::from(json!({ "markets": "BTC" })).rejected(),
        1
    );

    // Changes: a market is added once, replaced in place and removed; what
    // cannot be done says why, and a change forgets what was left out.
    let sol = perp("bybit", "SOL", "USDT");
    assert_eq!(
        settings.clone().adding(btc.clone()),
        Err(Refused::Duplicate)
    );
    assert_eq!(
        settings.clone().adding(spot_of(&sol)),
        Err(Refused::Unavailable)
    );
    let added = settings.clone().adding(sol.clone()).unwrap();
    assert_eq!(added.markets(), [btc.clone(), eth.clone(), sol.clone()]);
    assert_eq!(added.rejected(), 0);
    let replaced = added
        .clone()
        .replacing(&PairKey::of(&btc), perp("hyperliquid", "BTC", "USDC"))
        .unwrap();
    assert_eq!(replaced.markets()[0].exchange, "hyperliquid");
    assert_eq!(
        added.clone().replacing(&PairKey::of(&btc), eth.clone()),
        Err(Refused::Duplicate)
    );
    assert_eq!(
        replaced.clone().replacing(&PairKey::of(&btc), sol.clone()),
        Err(Refused::Unavailable)
    );
    let removed = added.clone().without(&PairKey::of(&eth)).unwrap();
    assert_eq!(removed.markets(), [btc, sol]);
    assert_eq!(
        removed.without(&PairKey::of(&eth)),
        Err(Refused::Unavailable)
    );
}

fn spot_of(market: &MarketSelection) -> MarketSelection {
    MarketSelection {
        kind: crate::data::ferris::MarketType::Spot,
        ..market.clone()
    }
}

/// What a stubbed exchange supports: catalogs, and statistics of its
/// perpetuals' funding.
fn stub_exchange(id: &str) -> Value {
    let every = json!({ "state": "supported", "supportedProducts": ["perp", "spot", "future"] });
    json!({
        "exchange": id,
        "features": { "fetchMarkets": every },
        "marketStats": {
            "state": "supported",
            "scope": { "exchange": id, "params": {} },
            "fields": { "perp": { "funding": { "state": "supported" } } },
            "pollIntervalMs": 5000,
            "staleAfterMs": 60000,
        },
    })
}

/// A market's statistics: its funding as the API restates it, received at
/// `received`.
fn stub_stats(market: Value, funding: Value, received: f64) -> Value {
    let mut market = market;
    market["fields"] = json!({ "funding": {
        "state": "available",
        "value": funding,
        "receivedTimestamp": received,
    } });
    market
}

fn bybit_funding() -> Value {
    json!({
        "rate": "0.0001",
        "equivalents": {
            "oneHourPercent": "0.00125",
            "eightHourPercent": "0.01",
            "oneDayPercent": "0.03",
            "annualizedPercent": "10.95",
        },
        "paymentIntervalMs": 28_800_000,
    })
}

fn hyperliquid_funding() -> Value {
    json!({
        "rate": "-0.0000125",
        "equivalents": {
            "oneHourPercent": "-0.00125",
            "eightHourPercent": "-0.01",
            "oneDayPercent": "-0.03",
            "annualizedPercent": "-0.000001",
        },
        "paymentIntervalMs": 3_600_000,
    })
}

/// The stubbed statistics of `exchange`, received at `received`.
fn stub_snapshot(exchange: &str, received: f64) -> Value {
    let markets = match exchange {
        "bybit" => vec![
            stub_stats(
                catalog_row("bybit", "BTC", "USDT", "perp"),
                bybit_funding(),
                received,
            ),
            stub_stats(
                catalog_row("bybit", "ETH", "USDT", "perp"),
                bybit_funding(),
                received,
            ),
        ],
        "hyperliquid" => vec![stub_stats(
            catalog_row("hyperliquid", "ETH", "USDC", "perp"),
            hyperliquid_funding(),
            received,
        )],
        _ => Vec::new(),
    };
    json!({
        "timestamp": received,
        "coverage": { "returnedMarkets": markets.len(), "enumerationComplete": true },
        "markets": markets,
    })
}

/// The feeds of the stubbed exchanges, each with its statistics received
/// at `received` and, for Bybit, `error` from its latest refresh.
fn snaps(received: f64, error: Option<ApiError>) -> Vec<FeedSnap> {
    let capabilities: Capabilities = serde_json::from_value(
        json!({ "exchanges": [stub_exchange("bybit"), stub_exchange("hyperliquid")] }),
    )
    .unwrap();
    stats_scopes(&capabilities, &[Product::Perp], &[StatsFieldName::Funding])
        .into_iter()
        .map(|scope| {
            let snapshot: StatsSnapshot =
                serde_json::from_value(stub_snapshot(&scope.exchange, received)).unwrap();
            let error = error.clone().filter(|_| scope.exchange == "bybit");
            FeedSnap::new(Rc::new(scope), Some(Rc::new(snapshot)), error)
        })
        .collect()
}

fn rows(markets: &[MarketSelection], snaps: &[FeedSnap], now: f64) -> Vec<Row> {
    session::rows_of(&session::ordered(markets), snaps, now)
}

fn rate(row: &Row, period: Period) -> Option<(&str, Sign)> {
    row.rate(period).map(|rate| (rate.text.as_str(), rate.sign))
}

#[test]
fn rows_show_the_rates_while_funding_is_usable_and_the_age_whatever_its_state() {
    const AT: f64 = 1_700_000_000_000.0;
    let markets = [
        perp("hyperliquid", "ETH", "USDC"),
        perp("bybit", "ETH", "USDT"),
        perp("bybit", "BTC", "USDT"),
        perp("bybit", "SOL", "USDT"),
    ];
    // By exchange name, then pair.
    let fresh = rows(&markets, &snaps(AT, None), AT + 5_900.0);
    let order: Vec<(&str, &str)> = fresh
        .iter()
        .map(|row| (row.market.exchange.as_str(), row.market.symbol.as_str()))
        .collect();
    assert_eq!(
        order,
        [
            ("bybit", "BTC/USDT"),
            ("bybit", "ETH/USDT"),
            ("bybit", "SOL/USDT"),
            ("hyperliquid", "ETH/USDC"),
        ]
    );

    // A positive rate, an interval in hours and an age in whole seconds.
    let btc = &fresh[0];
    assert_eq!(rate(btc, Period::Hour), Some(("0.00125%", Sign::Positive)));
    assert_eq!(
        rate(btc, Period::EightHours),
        Some(("0.01%", Sign::Positive))
    );
    assert_eq!(rate(btc, Period::Year), Some(("10.95%", Sign::Positive)));
    assert_eq!((btc.interval.as_str(), btc.age.as_str()), ("8h", "5s"));
    assert_eq!(btc.received, "2023-11-14T22:13:20.000Z");
    // A negative one, and one that rounds to zero and so has no sign.
    let eth = &fresh[3];
    assert_eq!(rate(eth, Period::Day), Some(("-0.03%", Sign::Negative)));
    assert_eq!(rate(eth, Period::Year), Some(("0%", Sign::Zero)));
    assert_eq!(eth.interval, "1h");
    // A market with no statistics shows nothing.
    let sol = &fresh[2];
    assert!(sol.rates.iter().all(Option::is_none));
    assert_eq!((sol.interval.as_str(), sol.age.as_str()), ("—", "—"));
    assert_eq!(sol.received, "—");

    // Past its exchange's freshness window the funding is stale and still
    // shows; so it is after a refresh failed.
    let old = rows(&markets, &snaps(AT, None), AT + 120_000.0);
    assert_eq!(
        rate(&old[0], Period::Hour),
        Some(("0.00125%", Sign::Positive))
    );
    assert_eq!(old[0].age, "120s");
    let error = ApiError::network("offline");
    let failed = rows(&markets, &snaps(AT, Some(error)), AT);
    assert_eq!(
        rate(&failed[0], Period::Hour),
        Some(("0.00125%", Sign::Positive))
    );

    // A field the API marks unavailable keeps its interval and age only.
    let mut unavailable = stub_snapshot("bybit", AT);
    unavailable["markets"][0]["fields"]["funding"]["state"] = json!("unavailable");
    let unavailable: Rc<StatsSnapshot> = Rc::new(serde_json::from_value(unavailable).unwrap());
    let feeds: Vec<FeedSnap> = snaps(AT, None)
        .into_iter()
        .map(|feed| match feed.scope.exchange.as_str() {
            "bybit" => FeedSnap::new(feed.scope, Some(unavailable.clone()), None),
            _ => feed,
        })
        .collect();
    let hidden = rows(&markets, &feeds, AT + 1_000.0);
    assert!(hidden[0].rates.iter().all(Option::is_none));
    assert_eq!(
        (hidden[0].interval.as_str(), hidden[0].age.as_str()),
        ("8h", "1s")
    );
}

/// A Ferris no test reaches: its HTTP is the test's stub network, and
/// nothing listens where its socket would connect.
const FERRIS: &str = "https://ferris.test";

/// A workspace of one Funding Rates widget watching Bybit's BTC/USDT and
/// Hyperliquid's ETH/USDC, with its data services as the shell provides
/// them.
#[component]
fn Harness() -> impl IntoElement {
    let services = DataServices::provide_with(FerrisConfig {
        http_base: FERRIS.to_owned(),
        ws_url: "ws://127.0.0.1:9/v1/ws".to_owned(),
        ..FerrisConfig::production()
    });
    provide_context(Clock::restore());
    let mut workspace = StoredWorkspace::default();
    layout::normalize(&mut workspace);
    layout::spawn(&mut workspace, WidgetKind::FundingRates);
    let widget = layout::widget_ids(&workspace)
        .into_iter()
        .next()
        .expect("the widget just added");
    let markets = [
        perp("hyperliquid", "ETH", "USDC"),
        perp("bybit", "BTC", "USDT"),
    ];
    let settings = WidgetSettings::read(WidgetKind::FundingRates, saved(&markets));
    assert!(layout::set_tab_settings(&mut workspace, widget, &settings));
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

/// Answers the stubbed Ferris's `POST`s: catalogs, and statistics received
/// five seconds ago.
fn stub_ferris(request: HttpRequest, reply: StubReply) {
    let body: Value = match &request.body {
        HttpBody::Bytes(bytes) => serde_json::from_slice(bytes).unwrap_or_default(),
        _ => Value::Null,
    };
    let exchange = body["exchange"].as_str().unwrap_or_default();
    let answer = if request.url.ends_with("/v1/fetchMarkets") {
        match exchange {
            "bybit" => json!([
                catalog_row("bybit", "BTC", "USDT", "perp"),
                catalog_row("bybit", "ETH", "USDT", "perp"),
                catalog_row("bybit", "BTC", "USDT", "spot"),
            ]),
            "hyperliquid" => json!([catalog_row("hyperliquid", "ETH", "USDC", "perp")]),
            _ => json!([]),
        }
    } else if request.url.ends_with("/v1/fetchMarketStats") {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |since| since.as_secs_f64() * 1000.0);
        stub_snapshot(exchange, now - 5_000.0)
    } else {
        json!([])
    };
    let mut response = StubResponse::default();
    response.status = 200;
    response.body = serde_json::to_vec(&answer).expect("the answer is JSON");
    reply.send(response);
}

/// Where a view with a test id is laid out: its left edge and its width.
fn span(app: &Headless, test_id: &str) -> (f32, f32) {
    let node = app
        .screen()
        .layout_nodes()
        .into_iter()
        .find(|node| node.test_id.as_deref() == Some(test_id))
        .unwrap_or_else(|| panic!("no view `{test_id}`"));
    (node.bounds.origin.x.0, node.bounds.size.width.0)
}

#[test]
fn the_table_shows_the_watched_markets_and_edits_them() {
    let capabilities =
        json!({ "exchanges": [stub_exchange("bybit"), stub_exchange("hyperliquid")] });
    let mut discovered = StubResponse::default();
    discovered.status = 200;
    discovered.body = serde_json::to_vec(&capabilities).expect("the capabilities are JSON");
    let network = Arc::new(StubNetwork::new([
        (
            format!("GET {FERRIS}/v1/capabilities"),
            StubRoute::Response(discovered),
        ),
        (
            format!("POST {FERRIS}/v1/*"),
            StubRoute::Handler(Arc::new(stub_ferris)),
        ),
    ]));
    let mut config = EngineConfig::new(Root::of::<Harness>());
    config.viewport = Size::new(px(1280.0), px(800.0));
    config.storage = Some(MemoryStorage::new([]));
    config.network = Some(network.clone());
    config.environment.os = Os::Windows;
    config.environment.idiom = Idiom::Desktop;
    let mut app = Headless::start(config).unwrap();

    // Both markets' rates arrive: the default columns, Bybit before
    // Hyperliquid.
    app.wait_for("0.00125%").unwrap();
    app.wait_for("-0.00125%").unwrap();
    for shown in [
        "Bybit",
        "Hyperliquid",
        "BTC/USDT",
        "ETH/USDC",
        "10.95%",
        "8h",
        "1h",
    ] {
        assert!(app.shows(shown), "{shown}");
    }
    for title in ["Exchange", "Pair", "1H", "1Y", "Interval", "Age", "X"] {
        assert!(app.shows(title), "{title}");
    }
    assert!(!app.shows("8H") && !app.shows("0.01%"));
    let aged = |screen: &nsh_testing::Screen| {
        let texts = screen.texts();
        texts
            .iter()
            .filter(|text| {
                text.strip_suffix('s')
                    .is_some_and(|seconds| seconds.parse::<u32>().is_ok())
            })
            .count()
    };
    assert_eq!(aged(app.screen()), 2);
    // Only the exchanges watched are polled, for funding alone.
    let polled: Vec<Value> = network
        .requests()
        .into_iter()
        .filter(|request| request.url.ends_with("/v1/fetchMarketStats"))
        .filter_map(|request| match &request.body {
            HttpBody::Bytes(bytes) => serde_json::from_slice(bytes).ok(),
            _ => None,
        })
        .collect();
    assert!(!polled.is_empty());
    assert!(
        polled
            .iter()
            .all(|body| body["fields"] == json!(["funding"]))
    );

    // The titles fill the pane side by side, × last and 42 points wide, and
    // the room left over goes to the columns in proportion to their content.
    let (table_left, table_width) = span(&app, "funding-rates-table");
    let titles = ["Exchange", "Pair", "1H", "1Y", "Interval", "Age", "X"]
        .map(|title| span(&app, &format!("funding-rates-title-{title}")));
    let mut edge = table_left;
    for (left, width) in titles {
        assert!((left - edge).abs() < 0.5, "{titles:?}");
        edge = left + width;
    }
    assert!(
        (edge - (table_left + table_width)).abs() < 0.5,
        "{titles:?}"
    );
    assert!((titles[6].1 - 42.0).abs() < 0.5, "{titles:?}");
    let (_, pane_width) = span(&app, "funding-rates");
    assert!((table_width - pane_width).abs() < 0.5);
    assert!(
        titles[1].1 > titles[0].1 && titles[0].1 > titles[5].1,
        "{titles:?}"
    );

    // The switches: 8H shows its column, the exchange's goes.
    app.press("Widget settings").unwrap();
    app.wait_for("id:funding-rates-settings").unwrap();
    app.press("label:Show 8H").unwrap();
    app.wait_for("8H").unwrap();
    assert!(app.shows("-0.01%"));
    app.press("label:Show Exchange").unwrap();
    app.wait_for_gone("Exchange", WAIT).unwrap();
    app.press("label:Close Funding Rates settings").unwrap();
    app.wait_for_gone("id:funding-rates-settings", WAIT)
        .unwrap();

    // "Add pair" offers the perpetuals of the exchanges that report
    // funding; one already watched is refused, and the widget says so.
    app.press("id:funding-rates-add-pair").unwrap();
    app.wait_for("id:market-picker").unwrap();
    app.wait_for("Bybit ETH/USDT PERP USDT-M").unwrap();
    assert_eq!(app.screen().count("role:option").unwrap(), 3);
    app.press("Bybit BTC/USDT PERP USDT-M").unwrap();
    app.wait_for_gone("id:market-picker", WAIT).unwrap();
    assert!(app.shows("Pair already added."));
    app.press("id:funding-rates-add-pair").unwrap();
    app.wait_for("Bybit ETH/USDT PERP USDT-M").unwrap();
    assert!(!app.shows("Pair already added."));
    app.press("Bybit ETH/USDT PERP USDT-M").unwrap();
    app.wait_for("label:Change pair: Bybit ETH/USDT").unwrap();
    app.wait_until("the new row's rates", WAIT, |screen| aged(screen) == 3)
        .unwrap();

    // A pair's cell changes it (its Clear removes it), and × removes one.
    app.press("label:Change pair: Hyperliquid ETH/USDC")
        .unwrap();
    app.wait_for("label:Remove pair").unwrap();
    app.press("label:Remove pair").unwrap();
    app.wait_for_gone("label:Change pair: Hyperliquid ETH/USDC", WAIT)
        .unwrap();
    app.press("label:Remove pair: Bybit ETH/USDT").unwrap();
    app.wait_for_gone("label:Change pair: Bybit ETH/USDT", WAIT)
        .unwrap();
    assert!(app.shows("BTC/USDT"));

    // A pane narrower than the columns keeps them whole and scrolls.
    app.set_viewport(Size::new(px(240.0), px(500.0))).unwrap();
    app.wait(std::time::Duration::from_millis(200)).unwrap();
    let (_, pane_width) = span(&app, "funding-rates");
    let (_, table_width) = span(&app, "funding-rates-table");
    assert!(
        table_width > pane_width + 50.0,
        "{table_width} in {pane_width}"
    );
    let titles = ["Pair", "1H", "8H", "1Y", "Interval", "Age", "X"]
        .map(|title| span(&app, &format!("funding-rates-title-{title}")).1);
    assert!((titles.iter().sum::<f32>() - table_width).abs() < 0.5);

    // With no market left the titles go, and "Add pair" stays.
    app.press("label:Remove pair: Bybit BTC/USDT").unwrap();
    app.wait_for_gone("Pair", WAIT).unwrap();
    assert!(app.shows("Add pair"));
}
