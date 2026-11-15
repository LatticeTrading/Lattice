use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use nsh_core::{
    MountItem,
    element::Root,
    platform::{Idiom, Os},
    prelude::*,
};
use nsh_engine::{EngineConfig, HttpBody, HttpRequest};
use nsh_testing::{Headless, MemoryStorage, StubNetwork, StubReply, StubResponse, StubRoute, WAIT};
use serde_json::{Value, json};

use super::settings::{ChartSettings, Interval};
use crate::{
    data::{
        DataServices,
        ferris::{FerrisConfig, MarketSelection, Timeframe},
    },
    layout,
    link::LinkColor,
    shell::clock::Clock,
    storage::StoredWorkspace,
    widgets::{WidgetKind, picker::Picker},
};

#[test]
fn settings_save_only_what_differs_and_read_leniently() {
    let saved = |settings: &ChartSettings| serde_json::to_value(settings).unwrap();
    assert_eq!(saved(&ChartSettings::default()), json!({}));
    assert_eq!(
        ChartSettings::default().market(),
        MarketSelection::bybit_btc_usdt()
    );
    assert_eq!(
        ChartSettings::default().interval(),
        Interval::Candles(Timeframe::M1)
    );

    let market = MarketSelection {
        exchange: "hyperliquid".into(),
        ..MarketSelection::bybit_btc_usdt()
    };
    let settings = ChartSettings::default()
        .with_market(Some(market.clone()))
        .with_interval(Interval::Tick)
        .with_grid(false);
    let value = saved(&settings);
    assert_eq!(value["timeframe"], "tick");
    assert_eq!(value["showGrid"], false);
    assert_eq!(value.get("showOhlcOnHover"), None);
    assert_eq!(ChartSettings::from(value), settings);

    // The old app's other spellings, and what it could not read.
    let read = |timeframe: Value| ChartSettings::from(json!({ "timeframe": timeframe })).interval();
    assert_eq!(read(json!("60")), Interval::Candles(Timeframe::H1));
    assert_eq!(read(json!("1T")), Interval::Tick);
    assert_eq!(read(json!("3m")), Interval::Candles(Timeframe::M1));
    assert_eq!(read(json!(5)), Interval::Candles(Timeframe::M1));
    let damaged = ChartSettings::from(json!({ "market": "BTC", "showGrid": "no" }));
    assert_eq!(damaged, ChartSettings::default());
}

#[test]
fn the_link_color_is_saved_only_when_it_is_not_the_default() {
    let saved = |settings: &ChartSettings| serde_json::to_value(settings).unwrap();
    // Blue until told otherwise, which saves nothing.
    let settings = ChartSettings::default();
    assert_eq!(settings.link(), Some(LinkColor::Blue));
    assert_eq!(
        saved(&settings.clone().with_link(Some(LinkColor::Blue))),
        json!({})
    );

    let red = settings.clone().with_link(Some(LinkColor::Red));
    assert_eq!(saved(&red), json!({ "linkColor": "red" }));
    assert_eq!(
        ChartSettings::from(saved(&red)).link(),
        Some(LinkColor::Red)
    );
    // No color is the old app's empty name.
    let none = settings.with_link(None);
    assert_eq!(saved(&none), json!({ "linkColor": "" }));
    assert_eq!(ChartSettings::from(saved(&none)).link(), None);
    assert_eq!(
        ChartSettings::from(json!({ "linkColor": null })).link(),
        None
    );

    // A color Lattice does not know, or no name at all, is the default.
    for odd in [json!("purple"), json!(5)] {
        let read = ChartSettings::from(json!({ "linkColor": odd }));
        assert_eq!(read, ChartSettings::default());
    }

    // A market sent to the chart's color is its market, and nothing else.
    let market = MarketSelection {
        exchange: "hyperliquid".into(),
        ..MarketSelection::bybit_btc_usdt()
    };
    let sent = red.clone().with_linked_market(market.clone());
    assert_eq!(sent.market(), market);
    assert_eq!(sent.link(), Some(LinkColor::Red));
    assert_eq!(sent.interval(), red.interval());
}

/// A Ferris no test reaches: its HTTP is the test's stub network, and
/// nothing listens where its socket would connect.
const FERRIS: &str = "https://ferris.test";

/// A workspace of one chart, with its data services as the shell provides
/// them.
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
    layout::spawn(&mut saved, WidgetKind::Chart);
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

/// The price a stubbed bar opens at: its number among all one-minute bars,
/// in a sawtooth from 1000 to 1099.
fn stub_price(bar: i64) -> f64 {
    1_000.0 + bar.rem_euclid(100) as f64
}

/// Answers `fetchOHLCV` with the one-minute bars of the range asked for,
/// at most a thousand from its start, as Ferris does for Bybit.
fn stub_candles(request: HttpRequest, reply: StubReply) {
    let body: Value = match &request.body {
        HttpBody::Bytes(bytes) => serde_json::from_slice(bytes).unwrap_or_default(),
        _ => Value::Null,
    };
    let ms = |name: &str| body[name].as_i64().unwrap_or(0);
    let first = (ms("since") + 59_999).div_euclid(60_000);
    let rows: Vec<Value> = (first..=ms("until").div_euclid(60_000))
        .take(1_000)
        .map(|bar| {
            let (open, close) = (stub_price(bar), stub_price(bar + 1));
            json!([
                bar * 60_000,
                open,
                open.max(close) + 1.0,
                open.min(close) - 1.0,
                close,
                1.0
            ])
        })
        .collect();
    let mut response = StubResponse::default();
    response.status = 200;
    response.body = serde_json::to_vec(&rows).expect("rows are JSON");
    reply.send(response);
}

/// The texts on screen that start with `prefix`.
fn texts_starting(app: &Headless, prefix: &str) -> Vec<String> {
    app.screen()
        .texts()
        .into_iter()
        .filter(|text| text.starts_with(prefix))
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_chart_loads_its_candles_and_follows_the_pointer() {
    let network = Arc::new(StubNetwork::new([(
        format!("POST {FERRIS}/v1/fetchOHLCV"),
        StubRoute::Handler(Arc::new(stub_candles)),
    )]));
    let asked = {
        let network = network.clone();
        move || {
            let histories = network.requests().into_iter();
            histories
                .filter(|request| request.url.ends_with("/v1/fetchOHLCV"))
                .count()
        }
    };
    let mut config = EngineConfig::new(Root::of::<Harness>());
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
    let pause = Duration::from_millis(200);

    // The 1m candles of Bybit's BTC/USDT, by default. Their history is
    // 1600 bars: the newest 500 in one reply, then the 1100 before them in
    // two, the first stopping at its thousandth bar.
    app.wait_for("BTC/USDT  ·  1m  ·  Bybit").unwrap();
    app.wait_for_gone("Loading historical candles...", WAIT)
        .unwrap();
    app.wait_for("1108.08").unwrap();
    app.wait_until("the rest of the history", WAIT, |_| asked() == 3)
        .unwrap();
    // The newest 220 show, their prices fitted: the stub's 999 to 1100
    // with 8% of room, a price beside each grid row.
    assert!(app.shows("990.920"));
    // The plot is 540 points wide here: two tiles of 110 slots, candles,
    // with the grid.
    assert_eq!(newest(1), Some(110.0));
    assert_eq!(newest(3), Some(0.0));
    assert_eq!(newest(5), Some(540.0));
    assert_eq!(newest(6), Some(7.0));

    // The crosshair names the bar under the pointer and its price.
    assert_eq!(texts_starting(&app, "O ").len(), 0);
    app.hover("id:chart-plot").unwrap();
    app.wait(pause).unwrap();
    let hovered = texts_starting(&app, "T ");
    assert_eq!(hovered.len(), 1);
    for prefix in ["O ", "H ", "L ", "C "] {
        assert_eq!(texts_starting(&app, prefix).len(), 1, "{prefix}");
    }
    // The middle of the plot is the middle of the fitted prices.
    assert!(app.shows("1049.50"));

    // A drag pans: the bar under the pointer stays under it, and both
    // tiles are drawn again as it goes.
    let drawn = layers.lock().unwrap().len();
    app.drag("id:chart-plot", Point::new(px(200.0), px(0.0)))
        .unwrap();
    app.wait(pause).unwrap();
    assert_eq!(texts_starting(&app, "T "), hovered);
    assert!(layers.lock().unwrap().len() > drawn);
    assert_eq!(newest(1), Some(110.0));

    // A drag on the time axis zooms: 150 points left shows e^(-150/220)
    // of the slots.
    app.drag("id:chart-time-axis", Point::new(px(-150.0), px(0.0)))
        .unwrap();
    app.wait(pause).unwrap();
    let per = newest(1).unwrap();
    assert!(
        (per - 110.0 * (-150.0_f32 / 220.0).exp()).abs() < 0.01,
        "{per}"
    );
    assert!(app.shows("1108.08"));

    // A drag on the price axis scales the prices about their middle (80
    // points down: e^0.5 of the span), which frees them...
    app.drag("id:chart-price-axis", Point::new(px(0.0), px(80.0)))
        .unwrap();
    app.wait(pause).unwrap();
    assert!(app.shows("1146.08") && app.shows("952.918"));
    // ...so a drag on the plot now moves them too...
    app.drag("id:chart-plot", Point::new(px(0.0), px(40.0)))
        .unwrap();
    app.wait(pause).unwrap();
    assert!(!app.shows("1146.08"));
    // ...until a double click on an axis fits them again.
    for _ in 0..2 {
        app.drag("id:chart-price-axis", Point::new(px(0.0), px(0.0)))
            .unwrap();
    }
    app.wait(pause).unwrap();
    assert!(app.shows("1108.08") && app.shows("990.920"));

    // Zoomed out past the oldest bar, the pages before it are asked for
    // while the pointer moves, and no more once it rests.
    app.drag("id:chart-time-axis", Point::new(px(700.0), px(0.0)))
        .unwrap();
    app.wait_until("a page of older candles", WAIT, |_| asked() > 3)
        .unwrap();
    app.wait_for_gone("Loading older candles...", WAIT).unwrap();
    app.wait(pause).unwrap();
    let pages = asked();
    app.wait(pause * 3).unwrap();
    assert_eq!(asked(), pages);
    assert!(!app.shows("Loading older candles..."));

    // The settings panel: the grid goes with its check.
    app.press("Widget settings").unwrap();
    app.wait_for("id:chart-settings").unwrap();
    app.press("Show Grid").unwrap();
    app.wait(pause).unwrap();
    assert_eq!(newest(6), Some(0.0));

    // Its select charts the timeframe picked.
    app.press("Timeframe").unwrap();
    app.press("5m").unwrap();
    app.wait_for("BTC/USDT  ·  5m  ·  Bybit").unwrap();
}
