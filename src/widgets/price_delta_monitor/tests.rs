use std::sync::Arc;

use nsh_core::{
    element::Root,
    platform::{Idiom, Os},
    prelude::*,
};
use nsh_engine::{EngineConfig, HttpBody, HttpRequest};
use nsh_testing::{Headless, MemoryStorage, StubNetwork, StubReply, StubResponse, StubRoute, WAIT};
use serde_json::{Value, json};

use super::{
    session::{Price, Tone, Watched, price_text, quote},
    settings::PriceDeltaSettings,
};
use crate::{
    data::{
        DataServices,
        ferris::{FerrisConfig, MarketSelection},
    },
    layout,
    shell::clock::Clock,
    storage::StoredWorkspace,
    widgets::{
        WidgetKind,
        common::books::testing::{FERRIS, book},
        picker::Picker,
    },
};

fn bybit() -> MarketSelection {
    MarketSelection::bybit_btc_usdt()
}

fn binance() -> MarketSelection {
    MarketSelection::binance_btc_usdt()
}

/// Bybit's ETH/USDT perpetual.
fn ether() -> MarketSelection {
    MarketSelection {
        symbol: "ETH/USDT".into(),
        base: "ETH".into(),
        request_symbol: "ETH/USDT:USDT".into(),
        exchange_symbol: "ETHUSDT".into(),
        ..bybit()
    }
}

#[test]
fn settings_save_only_what_differs_and_read_leniently() {
    let saved = |settings: &PriceDeltaSettings| serde_json::to_value(settings).unwrap();
    // Hyperliquid's BTC/USDC perpetual against Bybit's and Binance's
    // BTC/USDT.
    let defaults = PriceDeltaSettings::default();
    assert_eq!(saved(&defaults), json!({}));
    let base = defaults.base();
    assert_eq!(
        (base.exchange.as_str(), base.symbol.as_str(), base.tags()),
        ("hyperliquid", "BTC/USDC", "PERP USDC-M".to_owned())
    );
    assert_eq!(defaults.comparisons(), [bybit(), binance()]);
    // The defaults picked again save nothing.
    assert_eq!(saved(&defaults.clone().with_base(base.clone())), json!({}));

    // A market is compared once, and the base never is.
    let added = defaults.clone().adding(ether());
    assert_eq!(added.comparisons(), [bybit(), binance(), ether()]);
    assert_eq!(added.clone().adding(ether()), added);
    assert_eq!(added.clone().adding(base.clone()), added);
    let value = saved(&added);
    assert_eq!(value["comparisonMarkets"][2]["symbol"], "ETH/USDT");
    assert_eq!(value.get("baseMarket"), None);
    assert_eq!(PriceDeltaSettings::from(value), added);
    assert_eq!(added.comparison(&ether().identity()), Some(ether()));

    // A change puts a market where another was, unless it is taken.
    let changed = defaults.clone().replacing(&bybit().identity(), ether());
    assert_eq!(changed.comparisons(), [ether(), binance()]);
    for taken in [binance(), bybit(), base.clone()] {
        let kept = defaults.clone().replacing(&bybit().identity(), taken);
        assert_eq!(kept, defaults);
    }
    let gone = defaults.clone().replacing(&ether().identity(), ether());
    assert_eq!(gone, defaults);

    // An empty list is a choice.
    let none = defaults
        .clone()
        .without(&bybit().identity())
        .without(&binance().identity());
    assert!(none.comparisons().is_empty());
    assert_eq!(saved(&none), json!({ "comparisonMarkets": [] }));
    assert_eq!(PriceDeltaSettings::from(saved(&none)), none);

    // A compared market made the base is no longer compared.
    let based = defaults.clone().with_base(bybit());
    assert_eq!(
        (based.base(), based.comparisons()),
        (bybit(), vec![binance()])
    );
    let value = saved(&based);
    assert_eq!(value["baseMarket"]["exchange"], "bybit");
    assert_eq!(PriceDeltaSettings::from(value), based);

    // What cannot be read: the base and a list that is none are their
    // defaults, an entry that is no selection is dropped, and neither a
    // repeat nor the base is compared.
    let damaged = PriceDeltaSettings::from(json!({
        "baseMarket": "BTC",
        "comparisonMarkets": "all",
        "compare:okx": false,
    }));
    assert_eq!(damaged, defaults);
    let listed = PriceDeltaSettings::from(json!({
        "comparisonMarkets": [null, saved_market(&ether()), saved_market(&ether()), saved_market(&base)],
    }));
    assert_eq!(listed.comparisons(), [ether()]);
    assert_eq!(PriceDeltaSettings::from(json!("delta")), defaults);
}

fn saved_market(market: &MarketSelection) -> Value {
    serde_json::to_value(market).unwrap()
}

#[test]
fn prices_read_as_the_old_widget_wrote_them() {
    for (price, text) in [
        (104_235.5, "104,235.50"),
        (1_000_000.0, "1,000,000.00"),
        (1_234.567_8, "1,234.57"),
        (123.45678, "123.457"),
        (100.0, "100.00"),
        (1.23456, "1.2346"),
        (1.5, "1.50"),
        (0.5, "0.50"),
        (0.000_123_456, "0.000123"),
        (-1_234.5, "-1,234.50"),
    ] {
        assert_eq!(price_text(price), text);
    }
}

#[test]
fn rows_measure_each_market_against_the_base() {
    let elsewhere = MarketSelection {
        exchange: "binance".into(),
        ..ether()
    };
    let watched = [bybit(), binance(), ether(), elsewhere].map(Watched::of);
    let texts = |books: &[_], failed: &[bool]| {
        let (base, rows) = quote(&watched, books, failed);
        let rows = rows
            .iter()
            .map(|row| {
                (
                    row.price_text(),
                    row.difference_text(),
                    row.percent_text(),
                    row.tone(),
                )
            })
            .collect::<Vec<_>>();
        (base, rows)
    };
    let cells = |price: &str, difference: &str, percent: &str, tone| {
        (
            price.to_owned(),
            difference.to_owned(),
            percent.to_owned(),
            tone,
        )
    };

    // The base at 200: a market at 210 is above it, one with bids alone at
    // 150 below it, and one whose book never came and whose topic was
    // refused has no price.
    let books = [
        book(&[(199.0, 1.0)], &[(201.0, 1.0)]),
        book(&[(209.0, 1.0)], &[(211.0, 1.0)]),
        book(&[(150.0, 1.0)], &[]),
        None,
    ];
    let (base, rows) = texts(&books, &[false, false, false, true]);
    assert_eq!(base, Price::At(200.0));
    assert_eq!(
        rows,
        [
            cells("210.00", "+10.0000", "+5.000%", Some(Tone::Above)),
            cells("150.00", "-50.0000", "-25.000%", Some(Tone::Below)),
            cells("--", "--", "--", None),
        ]
    );
    assert_eq!(
        quote(&watched, &books, &[false; 4]).1[2].price,
        Price::Waiting
    );
    // A row is its market's, by its identity.
    let (_, made) = quote(&watched, &books, &[false; 4]);
    assert_eq!(made[1].key, ether().identity());
    assert_eq!(*made[1].market, ether());

    // The same price is level with the base; a refused book that came all
    // the same has its price.
    let level = [books[0].clone(), books[0].clone(), None, None];
    let (_, rows) = texts(&level, &[false, true, false, false]);
    assert_eq!(
        rows[0],
        cells("200.00", "+0.0000", "+0.000%", Some(Tone::Level))
    );
    assert_eq!(rows[1], cells("Waiting...", "--", "--", None));

    // Without the base's price nothing is measured; against a base at
    // zero there is a difference and no percentage.
    let (base, rows) = texts(&[None, books[1].clone()], &[true, false]);
    assert_eq!(base, Price::Unavailable);
    assert_eq!(rows[0], cells("210.00", "--", "--", None));
    let zero = [book(&[(0.0, 1.0)], &[(0.0, 1.0)]), books[1].clone()];
    let (_, rows) = texts(&zero, &[]);
    assert_eq!(
        rows[0],
        cells("210.00", "+210.0000", "--", Some(Tone::Above))
    );
}

/// A workspace of one Price Delta Monitor, with its data services as the
/// shell provides them.
#[component]
fn Harness() -> impl IntoElement {
    let services = DataServices::provide_with(FerrisConfig {
        http_base: FERRIS.to_owned(),
        ws_url: "ws://127.0.0.1:9/v1/ws".to_owned(),
        ..FerrisConfig::production()
    });
    provide_context(Clock::restore());
    let mut saved = StoredWorkspace::default();
    layout::normalize(&mut saved);
    layout::spawn(&mut saved, WidgetKind::PriceDeltaMonitor);
    let workspace = signal(saved);
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

/// A request's JSON body.
fn body(request: &HttpRequest) -> Value {
    match &request.body {
        HttpBody::Bytes(bytes) => serde_json::from_slice(bytes).unwrap_or_default(),
        _ => Value::Null,
    }
}

/// Answers `fetchOrderBook` with a book a point either side of 200 for
/// Hyperliquid, of 210 for Bybit, and of 200 for any other exchange.
fn stub_books(request: HttpRequest, reply: StubReply) {
    let mid = if body(&request)["exchange"] == "bybit" {
        210.0
    } else {
        200.0
    };
    let book = json!({
        "bids": [[mid - 1.0, 2.0]],
        "asks": [[mid + 1.0, 1.0]],
        "timestamp": 1_700_000_000_000_i64,
    });
    let mut response = StubResponse::default();
    response.status = 200;
    response.body = serde_json::to_vec(&book).expect("a book is JSON");
    reply.send(response);
}

/// Where a view with a test id is laid out: its left, top, width and
/// height.
fn bounds(app: &Headless, test_id: &str) -> [f32; 4] {
    let node = app
        .screen()
        .layout_nodes()
        .into_iter()
        .find(|node| node.test_id.as_deref() == Some(test_id))
        .unwrap_or_else(|| panic!("no view `{test_id}`"));
    let bounds = node.bounds;
    [
        bounds.origin.x.0,
        bounds.origin.y.0,
        bounds.size.width.0,
        bounds.size.height.0,
    ]
}

#[test]
fn a_monitor_shows_its_markets_against_the_base_and_edits_them() {
    let network = Arc::new(StubNetwork::new([(
        format!("POST {FERRIS}/v1/fetchOrderBook"),
        StubRoute::Handler(Arc::new(stub_books)),
    )]));
    // The books asked for, as their request bodies.
    let asked = {
        let network = network.clone();
        move || -> Vec<Value> {
            let requests = network.requests().into_iter();
            requests
                .filter(|request| request.url.ends_with("/v1/fetchOrderBook"))
                .map(|request| body(&request))
                .collect()
        }
    };
    let mut config = EngineConfig::new(Root::of::<Harness>());
    config.viewport = Size::new(px(1280.0), px(800.0));
    config.storage = Some(MemoryStorage::new([]));
    config.network = Some(network.clone());
    config.environment.os = Os::Windows;
    config.environment.idiom = Idiom::Desktop;
    let mut app = Headless::start(config).unwrap();

    // Hyperliquid's BTC/USDC against Bybit's and Binance's BTC/USDT by
    // default. Each price is the top of its market's book: one level a
    // side, asked for once per market.
    app.wait_for("+10.0000").unwrap();
    app.wait_for("+0.0000").unwrap();
    for shown in [
        "Hyperliquid",
        "BTC/USDC",
        "BTC/USDT",
        "200.00",
        "210.00",
        "+5.000%",
        "+0.000%",
        "Add comparison market",
    ] {
        assert!(app.shows(shown), "{shown}");
    }
    assert_eq!(asked().len(), 3);
    assert!(asked().iter().all(|book| book["limit"] == 1));
    // The tags are the segments, and no book is called indicative.
    for tag in ["PERP", "USDC-M", "USDT-M"] {
        assert!(app.shows(tag), "{tag}");
    }
    assert!(!app.shows("Indicative book"));
    assert!(!app.shows("Loading markets..."));
    // The base is a row over the table: its button at the left, as high
    // as a row, and its price out to the table's right edge.
    let (table, button, price) = (
        bounds(&app, "price-delta-monitor-table"),
        bounds(&app, "price-delta-monitor-base"),
        bounds(&app, "price-delta-monitor-base-price"),
    );
    assert_eq!((button[0], button[3], price[3]), (table[0], 41.0, 41.0));
    assert_eq!(button[0] + button[2], price[0]);
    assert_eq!(price[0] + price[2], table[0] + table[2]);
    assert_eq!(button[1] + button[3], table[1]);

    // × stops comparing a market, and lets go of its book.
    app.press("label:Remove Bybit BTC/USDT").unwrap();
    app.wait_for_gone("+10.0000", WAIT).unwrap();
    assert!(app.shows("+0.0000") && !app.shows("210.00"));

    // "Add comparison market", a row's market and the base open the market
    // picker; a row's has Clear, which removes it.
    app.press("id:price-delta-monitor-add").unwrap();
    app.wait_for("Search for symbol").unwrap();
    app.press_key("Escape").unwrap();
    app.wait_for_gone("Search for symbol", WAIT).unwrap();
    app.press("id:price-delta-monitor-base").unwrap();
    app.wait_for("Search for symbol").unwrap();
    assert!(!app.shows("label:Remove comparison"));
    app.press_key("Escape").unwrap();
    app.wait_for_gone("Search for symbol", WAIT).unwrap();
    app.press("label:Change comparison market: Binance BTC/USDT PERP USDT-M")
        .unwrap();
    app.wait_for("Search for symbol").unwrap();
    app.press("label:Remove comparison").unwrap();
    app.wait_for_gone("Search for symbol", WAIT).unwrap();
    app.wait_for_gone("+0.0000", WAIT).unwrap();
    assert!(app.shows("200.00") && app.shows("Add comparison market"));
}
