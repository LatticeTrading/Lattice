use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use nsh_core::{
    MountItem,
    platform::{Idiom, Os},
    prelude::*,
};
use nsh_engine::{EngineConfig, HttpBody, HttpRequest};
use nsh_testing::{Headless, MemoryStorage, StubNetwork, StubReply, StubResponse, StubRoute, WAIT};
use serde_json::{Value, json};

use super::{
    session::{Series, bar},
    settings::{Display, PremiumArbSettings},
    view::title,
};
use crate::{
    data::{
        DataServices,
        ferris::{FerrisConfig, MarketSelection, PairSample},
    },
    layout,
    shell::clock::Clock,
    storage::StoredWorkspace,
    widgets::{
        WidgetKind,
        common::books::{Pair, testing::FERRIS},
        picker::Picker,
    },
};

fn bybit() -> MarketSelection {
    MarketSelection::bybit_btc_usdt()
}

fn binance() -> MarketSelection {
    MarketSelection::binance_btc_usdt()
}

#[test]
fn settings_save_only_what_differs_and_read_leniently() {
    let saved = |settings: &PremiumArbSettings| serde_json::to_value(settings).unwrap();
    // Bybit's BTC/USDT against Binance's, the raw difference, the grid.
    let defaults = PremiumArbSettings::default();
    assert_eq!(saved(&defaults), json!({}));
    assert_eq!(defaults.markets(), Some((bybit(), binance())));
    assert_eq!(
        (defaults.display(), defaults.grid(), defaults.paired()),
        (Display::Raw, true, true)
    );
    // Each market's price is read from the top of its book.
    let layers = defaults.layers();
    assert_eq!(
        layers.iter().map(|layer| layer.pair).collect::<Vec<_>>(),
        Pair::ALL
    );
    assert!(layers[0].key.contains(r#""levels":1"#), "{}", layers[0].key);
    // The defaults picked again save nothing.
    let again = defaults
        .clone()
        .with_market(Pair::Secondary, Some(binance()))
        .with_display(Display::Raw)
        .with_grid(true);
    assert_eq!(saved(&again), json!({}));

    let elsewhere = MarketSelection {
        exchange: "hyperliquid".into(),
        ..bybit()
    };
    let settings = defaults
        .clone()
        .with_market(Pair::Secondary, Some(elsewhere.clone()))
        .with_display(Display::Percent)
        .with_grid(false);
    let value = saved(&settings);
    assert_eq!(value["displayMode"], "percent");
    assert_eq!(value["showGrid"], false);
    assert_eq!(value["secondaryMarket"]["exchange"], "hyperliquid");
    assert_eq!(value.get("primaryMarket"), None);
    assert_eq!(PremiumArbSettings::from(value), settings);
    assert_eq!(settings.markets(), Some((bybit(), elsewhere)));

    // A market cleared is saved as null, and leaves no pair to chart.
    let cleared = defaults.clone().with_market(Pair::Primary, None);
    assert_eq!(saved(&cleared), json!({ "primaryMarket": null }));
    let read = PremiumArbSettings::from(saved(&cleared));
    assert_eq!(read, cleared);
    assert!(!read.paired() && read.markets().is_none());
    assert_eq!(read.market(Pair::Secondary), Some(binance()));
    assert_eq!(read.layers().len(), 1);

    // What cannot be read: a market that is no selection is cleared, as
    // the old app cleared it; the rest are their defaults.
    let damaged = PremiumArbSettings::from(json!({
        "primaryMarket": "BTC",
        "displayMode": "ratio",
        "showGrid": "no",
    }));
    assert_eq!(damaged, cleared);
    assert_eq!(PremiumArbSettings::from(json!("premium")), defaults);
}

fn sample(time: f64, primary: f64, secondary: f64) -> PairSample {
    PairSample {
        time,
        primary,
        secondary,
    }
}

#[test]
fn a_series_plots_the_difference_raw_or_as_a_percentage() {
    let closes = |series: &Series| series.bars.iter().map(|bar| bar.close).collect::<Vec<_>>();
    let mut samples = VecDeque::from([sample(1.0, 100.0, 110.0)]);
    let mut series = Series::new(Display::Raw);
    assert_eq!(series.take(&samples, 1, Display::Raw), 1);
    assert_eq!(closes(&series), [10.0]);
    // Nothing new adds nothing.
    assert_eq!(series.take(&samples, 1, Display::Raw), 0);
    samples.push_back(sample(2.0, 200.0, 110.0));
    assert_eq!(series.take(&samples, 2, Display::Raw), 1);
    assert_eq!(closes(&series), [10.0, -90.0]);

    // The other mode makes every bar again, and adds none after the
    // newest. A bar is one value, at its sample's time.
    assert_eq!(series.take(&samples, 2, Display::Percent), 0);
    assert_eq!(closes(&series), [10.0, -45.0]);
    let newest = series.bars[1];
    assert_eq!(
        (newest.time, newest.open, newest.high, newest.low),
        (2.0, -45.0, -45.0, -45.0)
    );

    // A primary price of zero has no percentage, but it has a difference:
    // the sample is back when the raw mode is.
    samples.push_back(sample(3.0, 0.0, 10.0));
    assert_eq!(series.take(&samples, 3, Display::Percent), 0);
    assert_eq!(closes(&series), [10.0, -45.0]);
    assert_eq!(series.take(&samples, 3, Display::Raw), 0);
    assert_eq!(closes(&series), [10.0, -90.0, 10.0]);
    assert_eq!(bar(Display::Raw, &sample(4.0, f64::INFINITY, 1.0)), None);

    // The feed keeps only its newest samples; the series lets go of the
    // bars of the ones it dropped.
    samples.pop_front();
    samples.push_back(sample(4.0, 100.0, 104.0));
    assert_eq!(series.take(&samples, 4, Display::Raw), 1);
    assert_eq!(closes(&series), [-90.0, 10.0, 4.0]);
    // A series that starts late takes what the feed still keeps.
    let mut late = Series::new(Display::Percent);
    assert_eq!(late.take(&samples, 4, Display::Percent), 2);
    assert_eq!(closes(&late), [-45.0, 4.0]);
}

#[test]
fn the_header_names_what_is_compared() {
    assert_eq!(
        title(&bybit(), &binance(), Display::Raw),
        "BTC/USDT  ·  Tick  ·  Raw Delta | Binance vs Bybit"
    );
    // Two pairs: the second's, then the primary's.
    let ether = MarketSelection {
        symbol: "ETH/USDT".into(),
        ..binance()
    };
    assert_eq!(
        title(&bybit(), &ether, Display::Percent),
        "ETH/USDT - BTC/USDT  ·  Tick  ·  % Delta | Binance vs Bybit"
    );
}

/// A workspace of one Premium/Arb Chart, with its data services as the
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
    layout::spawn(&mut saved, WidgetKind::PremiumArbChart);
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
/// Bybit, and of 210 for any other exchange.
fn stub_books(request: HttpRequest, reply: StubReply) {
    let mid = if body(&request)["exchange"] == "bybit" {
        200.0
    } else {
        210.0
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

#[test]
fn a_premium_arb_chart_plots_its_pair_and_follows_its_settings() {
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
    let mut config = EngineConfig::new(nsh_testing::windowed_root::<Harness>());
    config.viewport = Size::new(px(1280.0), px(800.0));
    config.storage = Some(MemoryStorage::new([]));
    config.network = Some(network.clone());
    config.environment.os = Os::Windows;
    config.environment.idiom = Idiom::Desktop;
    let mut app = Headless::start(config).unwrap();
    // The uniforms of every tile drawn, newest last.
    let layers = Arc::new(Mutex::new(Vec::<Vec<f32>>::new()));
    {
        let layers = layers.clone();
        app.observe(move |_, batch| {
            let drawn = batch.items.iter().filter_map(|item| match item {
                MountItem::Shader {
                    layer: Some(layer), ..
                } => Some(
                    layer
                        .uniforms
                        .iter()
                        .map(|word| f32::from_bits(*word))
                        .collect(),
                ),
                _ => None,
            });
            layers.lock().unwrap().extend(drawn);
        });
    }
    let newest = |word: usize| {
        layers
            .lock()
            .unwrap()
            .last()
            .map(|words: &Vec<f32>| words[word])
    };

    // Bybit's BTC/USDT against Binance's by default, the raw difference.
    // Each price is the top of its market's book: one level a side.
    app.wait_for("BTC/USDT  ·  Tick  ·  Raw Delta | Binance vs Bybit")
        .unwrap();
    app.wait_until("both books", WAIT, |_| asked().len() == 2)
        .unwrap();
    assert!(asked().iter().all(|book| book["limit"] == 1));
    // 210 less 200: the newest difference's tag, and the middle of the
    // prices fitted to it.
    app.wait_for("10.000").unwrap();
    app.wait_for_gone("Waiting for both live prices...", WAIT)
        .unwrap();
    // The Chart's plot: a tile of a line, with the grid.
    assert_eq!((newest(3), newest(6)), (Some(1.0), Some(7.0)));

    // The settings panel: the same difference as a percentage of Bybit's
    // price, with the prices fitted again.
    app.press("Widget settings").unwrap();
    app.wait_for("id:premium-arb-chart-settings").unwrap();
    app.press("Display Mode").unwrap();
    app.press("% Delta").unwrap();
    app.wait_for("BTC/USDT  ·  Tick  ·  % Delta | Binance vs Bybit")
        .unwrap();
    app.wait_for("5.000").unwrap();
    assert!(!app.shows("10.000"));
    // The grid goes with its check.
    app.press("Show Grid").unwrap();
    app.wait_until("no grid", WAIT, |_| newest(6) == Some(0.0))
        .unwrap();

    // A market cleared in the picker leaves no pair: the chooser asks for
    // it, and its half opens the picker again.
    app.press("id:premium-arb-chart-market-secondary").unwrap();
    app.wait_for("Search for symbol").unwrap();
    app.press("Clear Pair 2").unwrap();
    app.wait_for_gone("Search for symbol", WAIT).unwrap();
    app.wait_for("Select second pair").unwrap();
    assert!(app.shows("Bybit BTC/USDT"));
    app.press("id:premium-arb-chart-pick-secondary").unwrap();
    app.wait_for("Search for symbol").unwrap();
    app.press_key("Escape").unwrap();
    app.wait_for_gone("Search for symbol", WAIT).unwrap();
    assert!(app.shows("Select second pair"));
}
