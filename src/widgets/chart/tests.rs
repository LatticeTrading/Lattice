use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use nsh_core::{
    MountItem, NodeId,
    element::Root,
    platform::{Idiom, Os},
    prelude::*,
};
use nsh_engine::{EngineConfig, HttpBody, HttpRequest};
use nsh_testing::{
    Headless, MemoryStorage, Screen, StubNetwork, StubReply, StubResponse, StubRoute, WAIT,
};
use serde_json::{Value, json};

use super::{
    drawings::{
        Anchor, Draft, Drawing, Emphasis, Grip, Ink, Item, Kind, Scale, hit, market_key, moved,
        paint,
    },
    indicators::{self, COLORS, Computed, Indicator},
    settings::{ChartSettings, Interval},
};
use crate::{
    data::{
        DataServices,
        ferris::{Candle, FerrisConfig, MarketSelection, Timeframe},
    },
    layout,
    link::LinkColor,
    shell::clock::Clock,
    storage::StoredWorkspace,
    widgets::{
        WidgetKind,
        common::{
            books::testing::{Record, same},
            plot::{Mode, Scene, Timeline, Viewport},
        },
        picker::Picker,
    },
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
    app.wait_for("BTC/USDT  ·  Bybit").unwrap();
    app.wait_for_gone("Loading historical candles...", WAIT)
        .unwrap();
    app.wait_for("1108.08").unwrap();
    app.wait_until("the rest of the history", WAIT, |_| asked() == 3)
        .unwrap();
    // The newest 220 show, their prices fitted: the stub's 999 to 1100
    // with 8% of room, a price beside each grid row.
    assert!(app.shows("990.920"));
    // The plot is 512 points wide here, beside the drawing toolbar's 28:
    // two tiles of 110 slots, candles, with the grid.
    assert_eq!(newest(1), Some(110.0));
    assert_eq!(newest(3), Some(0.0));
    assert_eq!(newest(5), Some(512.0));
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
    app.wait_for("label:Timeframe 5m").unwrap();
}

/// The point at `time` and `price`.
fn point(time: f64, price: f64) -> Anchor {
    Anchor::at(time, price).expect("a time and a price")
}

/// The same number but for rounding.
fn close(left: f64, right: f64) -> bool {
    (left - right).abs() < 1e-6
}

/// The same point but for rounding.
fn same_point(left: Anchor, right: Anchor) -> bool {
    close(left.time(), right.time()) && close(left.price(), right.price())
}

#[test]
fn drawings_are_saved_with_their_market_and_read_leniently() {
    let saved = |settings: &ChartSettings| serde_json::to_value(settings).unwrap();
    assert!(ChartSettings::default().drawings().is_empty());
    assert!(ChartSettings::default().tools());
    let market = market_key(&MarketSelection::bybit_btc_usdt());
    assert_eq!(market, "bybit:BTC/USDT:USDT");

    let points = vec![point(60.0, 100.0), point(120.5, 101.25)];
    let trend = Drawing::new(Kind::Trend, &market, points).unwrap();
    let settings = ChartSettings::default()
        .with_drawings(vec![trend])
        .with_tools(false);
    let value = saved(&settings);
    assert_eq!(
        value,
        json!({
            "drawings": [{
                "type": "trend",
                "market": "bybit:BTC/USDT:USDT",
                "points": [[60.0, 100.0], [120.5, 101.25]],
            }],
            "showDrawingTools": false,
        })
    );
    assert_eq!(ChartSettings::from(value), settings);

    // A drawing has its kind's points, and a point is two numbers.
    assert!(Drawing::new(Kind::Fib, &market, vec![point(1.0, 2.0)]).is_none());
    assert!(Anchor::at(f64::NAN, 1.0).is_none());
    // What is no drawing is left out: a kind Lattice does not know or
    // does not keep, too few points, no market, a price that is no number.
    let damaged = ChartSettings::from(json!({
        "drawings": [
            { "type": "trend", "market": "m", "points": [[1, 2], [3, 4]] },
            { "type": "arrow", "market": "m", "points": [[1, 2], [3, 4]] },
            { "type": "measure", "market": "m", "points": [[1, 2], [3, 4]] },
            { "type": "fib", "market": "m", "points": [[1, 2]] },
            { "type": "fib", "market": "", "points": [[1, 2], [3, 4]] },
            { "type": "fib", "market": "m", "points": [[1, "2"], [3, 4]] },
            "trend",
        ],
        "showDrawingTools": "no",
    }));
    let [kept] = damaged.drawings() else {
        panic!("one drawing reads: {:?}", damaged.drawings());
    };
    assert_eq!((kept.kind(), kept.market()), (Kind::Trend, "m"));
    assert_eq!(kept.points(), [point(1.0, 2.0), point(3.0, 4.0)]);
    assert!(damaged.tools());
    let none = ChartSettings::from(json!({ "drawings": "none" }));
    assert_eq!(none, ChartSettings::default());
}

/// A hundred one-minute bars from time 0, each from 100 to 200, and the
/// scene of a plot of 500 by 116 points showing the newest fifty: ten
/// points a slot from slot 50, and a point a price down from 208 (the
/// bars' range with its 8% of room).
fn plotted() -> (Vec<Candle>, Scene) {
    let bars: Vec<Candle> = (0..100)
        .map(|bar| Candle {
            time: f64::from(bar) * 60.0,
            open: 100.0,
            high: 200.0,
            low: 100.0,
            close: 200.0,
            volume: None,
        })
        .collect();
    let scene = Scene::build(&bars, &Viewport::showing(50.0), Mode::Candles, 500.0, 116.0);
    (bars, scene)
}

/// Colors that tell what drew what.
fn ink() -> Ink {
    let shade = |hue: f32| hsla(hue, 1.0, 0.5, 1.0);
    Ink {
        line: shade(0.0),
        dim: shade(0.1),
        text: shade(0.2),
        panel: shade(0.3),
        edge: shade(0.4),
        ground: shade(0.5),
        up: shade(0.6),
        down: shade(0.7),
        up_tint: shade(0.8),
        down_tint: shade(0.9),
    }
}

#[test]
fn a_scale_places_points_by_their_times_and_prices() {
    let (bars, scene) = plotted();
    let scale = Scale::new(&scene, Timeline::new(&bars, Some(60.0)));
    let spot = scale.spot(point(3_600.0, 158.0)).unwrap();
    assert!(close(spot.x, 100.0) && close(spot.y, 50.0) && close(spot.slot, 60.0));
    let under = scale.anchor(300.0, 100.0).unwrap();
    assert!(same_point(under, point(4_800.0, 108.0)));
    // Past the newest bar and off the plot, time and price go on.
    let ahead = scale.spot(point(7_200.0, 300.0)).unwrap();
    assert!(close(ahead.x, 700.0) && close(ahead.y, -92.0));

    // On another interval's bars a point is over the bar of its time:
    // five-minute bars, fifty points a slot from slot 10.
    let wider: Vec<Candle> = bars.iter().step_by(5).copied().collect();
    let scene = Scene::build(
        &wider,
        &Viewport::showing(10.0),
        Mode::Candles,
        500.0,
        116.0,
    );
    let scale = Scale::new(&scene, Timeline::new(&wider, Some(300.0)));
    assert!(close(scale.spot(point(3_600.0, 158.0)).unwrap().x, 100.0));
    assert!(close(scale.spot(point(3_750.0, 158.0)).unwrap().x, 125.0));

    // Without a bar a point is nowhere, and nowhere is a point.
    let empty = Scale::new(&scene, Timeline::new(&[], Some(300.0)));
    assert!(empty.spot(point(3_600.0, 158.0)).is_none());
    assert!(empty.anchor(100.0, 50.0).is_none());
}

#[test]
fn each_kind_is_painted_where_its_points_are() {
    let (bars, scene) = plotted();
    let scale = Scale::new(&scene, Timeline::new(&bars, Some(60.0)));
    let ink = ink();
    // From (100, 50) to (300, 100) on the plot.
    let (from, to) = (point(3_600.0, 158.0), point(4_800.0, 108.0));
    let painted = |kind: Kind, emphasis: Emphasis| {
        let item = Item {
            kind,
            points: vec![from, to],
            emphasis,
        };
        Record::of(&paint(&scale, &[item], &ink).finish())
    };
    let starts_at =
        |line: (f32, f32), x: f32, y: f32| (line.0 - x).abs() < 1e-3 && (line.1 - y).abs() < 1e-3;

    // A trend line: one line, and a handle on each point once picked.
    let trend = painted(Kind::Trend, Emphasis::Plain);
    assert_eq!((trend.lines.len(), trend.boxes.len()), (1, 0));
    assert!(starts_at(trend.lines[0].from, 100.0, 50.0));
    assert!(!trend.lines[0].dashed && trend.texts.is_empty());
    let picked = painted(Kind::Trend, Emphasis::Picked);
    assert_eq!((picked.lines.len(), picked.boxes.len()), (1, 2));
    let [left, top, width, height] = picked.boxes[0].0;
    assert!((left - 96.5).abs() <= 0.5 && (top - 46.5).abs() <= 0.5);
    assert_eq!((width, height), (7.0, 7.0));

    // A retracement: the dashed line between its points, then a line and
    // a name a level, from all of the move retraced down to none.
    let fib = painted(Kind::Fib, Emphasis::Plain);
    assert_eq!(fib.lines.len(), 8);
    assert!(fib.lines[0].dashed && starts_at(fib.lines[0].from, 100.0, 50.0));
    let levels = &fib.lines[1..];
    assert!(
        levels
            .iter()
            .all(|line| !line.dashed && (line.from.0 - 100.0).abs() < 1e-3)
    );
    assert_eq!(
        fib.words(),
        [
            "0 (108.000)",
            "0.236 (119.800)",
            "0.382 (127.100)",
            "0.5 (133.000)",
            "0.618 (138.900)",
            "0.786 (147.300)",
            "1 (158.000)",
        ]
    );

    // A measurement: a tinted box between its points, an arrow across it
    // and one down it, and what it measured.
    let measure = painted(Kind::Measure, Emphasis::Plain);
    assert_eq!(measure.lines.len(), 6);
    assert_eq!(measure.words(), ["-50.000 (-31.65%)", "20 bars, 20m"]);
    let (tint, _) = measure.boxes[0];
    assert!(close(f64::from(tint[0]), 100.0) && close(f64::from(tint[2]), 200.0));

    // A point months off the plot asks nothing of the canvas beyond it.
    let far = Item {
        kind: Kind::Trend,
        points: vec![point(-90_000_000.0, 158.0), to],
        emphasis: Emphasis::Plain,
    };
    let far = Record::of(&paint(&scale, &[far], &ink).finish());
    assert!(far.lines[0].from.0 >= -8.001);
}

#[test]
fn a_press_takes_a_point_or_a_line_and_a_drag_moves_what_it_took() {
    let (bars, scene) = plotted();
    let scale = Scale::new(&scene, Timeline::new(&bars, Some(60.0)));
    // A trend line from (100, 50) to (300, 100), and over it a
    // retracement from (200, 40) to (400, 90).
    let (from, to) = (point(3_600.0, 158.0), point(4_800.0, 108.0));
    let trend = Drawing::new(Kind::Trend, "m", vec![from, to]).unwrap();
    let ends = vec![point(4_200.0, 168.0), point(5_400.0, 118.0)];
    let fib = Drawing::new(Kind::Fib, "m", ends).unwrap();
    let drawings = [(0, &trend), (1, &fib)];
    let takes = |at: (f64, f64)| hit(&scale, &drawings, None, at);

    assert_eq!(takes((103.0, 52.0)), Some((0, Grip::Point(0))));
    assert_eq!(takes((160.0, 66.0)), Some((0, Grip::Body)));
    assert_eq!(takes((30.0, 110.0)), None);
    // A level of the retracement is its line as much as its diagonal.
    assert_eq!(takes((350.0, 66.0)), Some((1, Grip::Body)));
    assert_eq!(takes((300.0, 66.0)), Some((1, Grip::Body)));
    // Where two cross, the one on top is taken...
    assert_eq!(takes((250.0, 88.5)), Some((1, Grip::Body)));
    // ...but a point of the picked one before any other's.
    let other = Drawing::new(Kind::Trend, "m", vec![from, point(5_400.0, 198.0)]).unwrap();
    let stacked = [(0, &trend), (1, &other)];
    assert_eq!(
        hit(&scale, &stacked, None, (100.0, 50.0)),
        Some((1, Grip::Point(0)))
    );
    assert_eq!(
        hit(&scale, &stacked, Some(0), (100.0, 50.0)),
        Some((0, Grip::Point(0)))
    );

    // A point taken moves alone, by as much as the pointer did.
    let grabbed = scale.anchor(100.0, 50.0).unwrap();
    let pulled = moved(
        &scale,
        trend.points(),
        Grip::Point(0),
        grabbed,
        (120.0, 70.0),
    )
    .unwrap();
    assert!(same_point(pulled[0], point(3_720.0, 138.0)));
    assert_eq!(pulled[1], to);
    // The line taken moves whole.
    let grabbed = scale.anchor(200.0, 75.0).unwrap();
    let carried = moved(&scale, trend.points(), Grip::Body, grabbed, (210.0, 65.0)).unwrap();
    assert!(same_point(carried[0], point(3_660.0, 168.0)));
    assert!(same_point(carried[1], point(4_860.0, 118.0)));
}

#[test]
fn a_draft_places_its_points_one_after_another() {
    let (first, second) = (point(60.0, 100.0), point(120.0, 110.0));
    let mut draft = Draft::begin(Kind::Trend, first);
    // Its first point is placed; the next follows the pointer from there.
    assert_eq!(draft.points(), [first, first]);
    draft.aim(second);
    assert_eq!(draft.points(), [first, second]);
    assert_eq!(draft.place(), Some(vec![first, second]));
}

/// What the drawings' canvas showed when it was last painted: where each
/// of its lines starts, how many rectangles it filled, and its texts.
#[derive(Clone, Default)]
struct Painted {
    lines: Vec<(f32, f32)>,
    boxes: usize,
    texts: Vec<String>,
}

/// Where layout put the view whose test id is `id`, in the window.
fn placed(app: &Headless, id: &str) -> Option<Bounds<Points>> {
    let nodes = app.screen().layout_nodes().into_iter();
    nodes
        .filter(|node| node.test_id.as_deref() == Some(id))
        .map(|node| node.bounds)
        .next()
}

/// The member `name` of the first object of `value` that has one.
fn member<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    match value {
        Value::Object(members) => members
            .get(name)
            .or_else(|| members.values().find_map(|inner| member(inner, name))),
        Value::Array(items) => items.iter().find_map(|inner| member(inner, name)),
        _ => None,
    }
}

#[test]
fn drawings_are_made_picked_moved_and_removed_on_the_plot() {
    let network = Arc::new(StubNetwork::new([(
        format!("POST {FERRIS}/v1/fetchOHLCV"),
        StubRoute::Handler(Arc::new(stub_candles)),
    )]));
    let storage = MemoryStorage::new([]);
    let mut config = EngineConfig::new(Root::of::<Harness>());
    config.viewport = Size::new(px(1280.0), px(800.0));
    config.storage = Some(storage.clone());
    config.network = Some(network);
    config.environment.os = Os::Windows;
    config.environment.idiom = Idiom::Desktop;
    let mut app = Headless::start(config).unwrap();
    let shown = Arc::new(Mutex::new(Painted::default()));
    // The plot's width, as the tile drawn last was told it.
    let plot = Arc::new(Mutex::new(None::<f32>));
    {
        let (shown, plot) = (shown.clone(), plot.clone());
        app.observe(move |_, batch| {
            for item in &batch.items {
                match item {
                    MountItem::Canvas { drawing, .. } => {
                        let record = Record::of(drawing);
                        *shown.lock().unwrap() = Painted {
                            lines: record.lines.iter().map(|line| line.from).collect(),
                            boxes: record.boxes.len(),
                            texts: record.words().into_iter().map(str::to_owned).collect(),
                        };
                    }
                    MountItem::Shader {
                        layer: Some(layer), ..
                    } => {
                        *plot.lock().unwrap() =
                            layer.uniforms.get(5).map(|word| f32::from_bits(*word));
                    }
                    _ => {}
                }
            }
        });
    }
    let painted = move || shown.lock().unwrap().clone();
    let plot_width = move || *plot.lock().unwrap();
    // What the chart's tab saved under `name`.
    let saved = move |name: &str| -> Option<Value> {
        let entries = storage.entries();
        let workspace: Value = serde_json::from_str(entries.get("lattice.workspace.v1")?).ok()?;
        member(&workspace, name).cloned()
    };
    let drawings = {
        let saved = saved.clone();
        move || -> Vec<Value> {
            let drawings = saved("drawings");
            drawings
                .and_then(|drawings| drawings.as_array().cloned())
                .unwrap_or_default()
        }
    };
    let pause = Duration::from_millis(200);
    let by = |x: f32, y: f32| Point::new(px(x), px(y));

    app.wait_for("BTC/USDT  ·  Bybit").unwrap();
    app.wait_for_gone("Loading historical candles...", WAIT)
        .unwrap();
    app.wait_for("1108.08").unwrap();

    // With the trend line's tool in hand, a drag on the plot (from its
    // middle) draws one instead of panning: it is saved on the market
    // charted, and picked, a handle on each of its points.
    app.press("Trend line").unwrap();
    app.drag("id:chart-plot", by(120.0, -60.0)).unwrap();
    app.wait_until("the trend line", WAIT, |_| drawings().len() == 1)
        .unwrap();
    app.wait(pause).unwrap();
    let made = drawings();
    assert_eq!(made[0]["type"], "trend");
    assert_eq!(made[0]["market"], "bybit:BTC/USDT:USDT");
    let line = painted();
    assert_eq!((line.lines.len(), line.boxes), (1, 2));
    // The plot is 512 points wide: its middle is where the drag began.
    let start = line.lines[0];
    assert!((start.0 - 256.0).abs() < 0.01, "{start:?}");

    // Escape unpicks it, and a click on it picks it again.
    app.press_key("Escape").unwrap();
    app.wait_until("no handles", WAIT, |_| painted().boxes == 0)
        .unwrap();
    app.drag("id:chart-plot", by(0.0, 0.0)).unwrap();
    app.wait_until("its handles", WAIT, |_| painted().boxes == 2)
        .unwrap();
    assert_eq!(drawings(), made);

    // A drag from its first point moves that point with the pointer and
    // leaves the other where it was.
    app.drag("id:chart-plot", by(40.0, 30.0)).unwrap();
    app.wait_until("the moved point", WAIT, |_| drawings() != made)
        .unwrap();
    app.wait(pause).unwrap();
    let moved = drawings();
    assert_eq!(moved.len(), 1);
    assert_ne!(moved[0]["points"][0], made[0]["points"][0]);
    assert_eq!(moved[0]["points"][1], made[0]["points"][1]);
    let end = painted().lines[0];
    assert!((end.0 - start.0 - 40.0).abs() < 0.01 && (end.1 - start.1 - 30.0).abs() < 0.01);

    // Delete removes the picked drawing.
    app.press_key("Delete").unwrap();
    app.wait_until("no drawing", WAIT, |_| drawings().is_empty())
        .unwrap();
    app.wait_until("an empty canvas", WAIT, |_| painted().lines.is_empty())
        .unwrap();

    // A measurement says what is between its points (80 points left of
    // the middle and 50 under it: older, and lower), is not saved, and
    // goes with the next press.
    app.press("Measure").unwrap();
    app.drag("id:chart-plot", by(-80.0, 50.0)).unwrap();
    app.wait_until("the measurement", WAIT, |_| painted().texts.len() == 2)
        .unwrap();
    let said = painted().texts;
    assert!(
        said[0].starts_with('-') && said[0].ends_with("%)"),
        "{said:?}"
    );
    assert!(said[1].starts_with("34 bars, 34m"), "{said:?}");
    assert!(drawings().is_empty());
    app.drag("id:chart-plot", by(0.0, 0.0)).unwrap();
    app.wait_until("no measurement", WAIT, |_| painted().texts.is_empty())
        .unwrap();

    // Escape puts a tool away: the drag after it draws nothing.
    app.press("Trend line").unwrap();
    app.press_key("Escape").unwrap();
    app.drag("id:chart-plot", by(60.0, 0.0)).unwrap();
    app.wait(pause).unwrap();
    assert!(drawings().is_empty() && painted().lines.is_empty());

    // A retracement by two presses: a click places its first point, and
    // the next press, wherever it is let go, its second.
    app.press("Fib retracement").unwrap();
    app.drag("id:chart-plot", by(0.0, 0.0)).unwrap();
    app.wait(pause).unwrap();
    assert!(drawings().is_empty());
    app.drag("id:chart-plot", by(100.0, 80.0)).unwrap();
    app.wait_until("the retracement", WAIT, |_| drawings().len() == 1)
        .unwrap();
    app.wait(pause).unwrap();
    assert_eq!(drawings()[0]["type"], "fib");
    let levels = painted();
    assert_eq!((levels.lines.len(), levels.texts.len()), (8, 7));
    assert!(levels.texts[0].starts_with("0 (") && levels.texts[6].starts_with("1 ("));

    // Removing the market's drawings asks twice.
    app.press("Remove drawings").unwrap();
    app.wait_for("Remove 1 drawing?").unwrap();
    assert_eq!(drawings().len(), 1);
    app.press("Remove 1 drawing?").unwrap();
    app.wait_until("no drawings", WAIT, |_| drawings().is_empty())
        .unwrap();

    // The toolbar is 28 points of the chart's 626 while it is open, the
    // button that closes it in its bottom corner, and the plot's area
    // starts 12 points past it.
    let chart = placed(&app, "chart").expect("the chart");
    let toolbar = placed(&app, "chart-tools").expect("the toolbar");
    let close = placed(&app, "chart-tools-hide").expect("its bottom button");
    let beside = placed(&app, "chart-plot").expect("the plot");
    assert_eq!(chart.size.width.0, 626.0);
    assert_eq!(toolbar.size.width.0, 28.0);
    assert_eq!(beside.origin.x.0 - chart.origin.x.0, 40.0);
    assert_eq!(toolbar.size.height, chart.size.height);
    assert_eq!(close.origin.x, chart.origin.x);
    assert_eq!(close.bottom(), chart.bottom());
    assert_eq!(plot_width(), Some(512.0));
    assert!(!app.screen().matches("Show drawing tools").unwrap());

    // Closed (which is saved), it takes no room: the plot has its width
    // too, and its tools are gone.
    assert_eq!(saved("showDrawingTools"), None);
    app.press("Hide drawing tools").unwrap();
    app.wait_for("Show drawing tools").unwrap();
    assert_eq!(saved("showDrawingTools"), Some(json!(false)));
    app.wait_until("the plot's whole width", WAIT, |_| {
        plot_width() == Some(540.0)
    })
    .unwrap();
    for gone in ["Hide drawing tools", "Trend line", "Remove drawings"] {
        assert!(!app.screen().matches(gone).unwrap(), "{gone}");
    }
    // Nothing beside the plot is the toolbar's then: the plot's area
    // starts at its own inset from the chart's left edge.
    let surface = placed(&app, "chart-plot").expect("the plot");
    assert_eq!(surface.origin.x.0 - chart.origin.x.0, 12.0);
    assert_eq!(surface.size.width.0, 540.0);
    // The badge that opens it floats on the plot: thin, all of it over
    // the plot's area, 4 points inside its left edge and halfway down it.
    let badge = placed(&app, "chart-tools-show").expect("the badge");
    assert_eq!((badge.size.width.0, badge.size.height.0), (10.0, 20.0));
    assert_eq!(badge.origin.x.0 - surface.origin.x.0, 4.0);
    assert!(badge.origin.y >= surface.origin.y && badge.bottom() <= surface.bottom());
    assert!(badge.right() <= surface.right());
    assert!((badge.center().y.0 - surface.center().y.0).abs() <= 0.5);

    // A press on the badge brings the toolbar out again.
    app.press("Show drawing tools").unwrap();
    app.wait_for("Hide drawing tools").unwrap();
    assert_eq!(saved("showDrawingTools"), None);
    app.wait_until("the plot beside the toolbar", WAIT, |_| {
        plot_width() == Some(512.0)
    })
    .unwrap();
    assert!(app.screen().matches("Trend line").unwrap());
    assert!(!app.screen().matches("Show drawing tools").unwrap());
}

/// One-minute bars from time 0, each all at its close of `closes`.
fn closing(closes: &[f64]) -> Vec<Candle> {
    let bars = closes.iter().enumerate();
    bars.map(|(bar, close)| Candle {
        time: bar as f64 * 60.0,
        open: *close,
        high: *close,
        low: *close,
        close: *close,
        volume: None,
    })
    .collect()
}

/// A `kind` of indicator computed with `inputs`, one a input of its kind.
fn indicator(kind: indicators::Kind, inputs: &[f64]) -> Indicator {
    let inputs = inputs.iter().enumerate();
    inputs.fold(Indicator::new(kind, &[]), |indicator, (at, value)| {
        indicator.with_input(at, *value)
    })
}

/// Whether `line` has `values` at the bars from the first on, but for
/// rounding.
fn runs(line: &indicators::Line, values: &[Option<f64>]) -> bool {
    values
        .iter()
        .enumerate()
        .all(|(bar, value)| match (line.at(bar), value) {
            (Some(left), Some(right)) => close(left, *right),
            (None, None) => true,
            _ => false,
        })
}

#[test]
fn indicators_are_saved_with_their_inputs_and_read_leniently() {
    use indicators::Kind;
    let saved = |settings: &ChartSettings| serde_json::to_value(settings).unwrap();
    assert!(ChartSettings::default().indicators().is_empty());

    // One starts with its kind's inputs as they start, showing, in the
    // first color no other has.
    let ema = Indicator::new(Kind::Ema, &[]);
    assert_eq!(ema.title(), "EMA 20");
    assert_eq!((ema.color(), ema.hidden()), (LinkColor::Yellow, false));
    let bands = Indicator::new(Kind::Bands, std::slice::from_ref(&ema))
        .with_input(1, 2.5)
        .with_hidden(true);
    assert_eq!(bands.title(), "BB 20 2.5");
    assert_eq!(bands.color(), LinkColor::Blue);
    let settings = ChartSettings::default().with_indicators(vec![ema.clone(), bands]);
    let value = saved(&settings);
    assert_eq!(
        value,
        json!({
            "indicators": [
                { "type": "ema", "length": 20, "color": "yellow" },
                { "type": "bb", "length": 20, "mult": 2.5, "color": "blue", "hidden": true },
            ],
        })
    );
    assert_eq!(ChartSettings::from(value), settings);

    // Once every color is taken they go round again.
    let mut five: Vec<Indicator> = Vec::new();
    for _ in COLORS {
        let next = Indicator::new(Kind::Ema, &five);
        five.push(next);
    }
    let colors: Vec<LinkColor> = five.iter().map(Indicator::color).collect();
    assert_eq!(colors, COLORS);
    assert_eq!(Indicator::new(Kind::Tema, &five).color(), COLORS[0]);

    // An input takes the nearest of its values, and what it starts as
    // for no number.
    let length = |value: f64| ema.clone().with_input(0, value).inputs()[0];
    assert_eq!((length(0.0), length(20.4), length(1e9)), (1.0, 20.0, 500.0));
    assert_eq!(length(f64::NAN), 20.0);

    // What is no indicator is left out (a kind Lattice does not know),
    // and one that is reads with what it can: an input of the wrong type
    // as it starts, one out of its range as the nearest in it.
    let damaged = ChartSettings::from(json!({
        "indicators": [
            { "type": "ema", "length": "9", "color": "purple" },
            { "type": "tema", "length": 9000.4, "color": "red", "hidden": "yes" },
            { "type": "rsi", "length": 14 },
            { "type": "bb", "length": 0, "mult": 2.2, "hidden": true },
            "ema",
        ],
    }));
    let read = damaged.indicators();
    let titles: Vec<String> = read.iter().map(Indicator::title).collect();
    assert_eq!(titles, ["EMA 20", "TEMA 500", "BB 1 2"]);
    let colors: Vec<LinkColor> = read.iter().map(Indicator::color).collect();
    assert_eq!(
        colors,
        [LinkColor::Yellow, LinkColor::Red, LinkColor::Yellow]
    );
    let hidden: Vec<bool> = read.iter().map(Indicator::hidden).collect();
    assert_eq!(hidden, [false, false, true]);
    let none = ChartSettings::from(json!({ "indicators": "ema" }));
    assert_eq!(none, ChartSettings::default());
}

#[test]
fn indicators_are_computed_from_the_closes() {
    use indicators::Kind;
    let rising = closing(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]);

    // An average over three bars starts at the third, as their mean, and
    // then goes half the way to each close: a bar behind a steady rise.
    let ema = indicator(Kind::Ema, &[3.0]).lines(&rising);
    let [average] = ema.as_slice() else {
        panic!("an average is one line: {ema:?}");
    };
    let behind = [None, None, Some(2.0), Some(3.0), Some(4.0), Some(5.0)];
    assert!(runs(average, &behind) && average.at(6) == Some(6.0));
    assert!(!average.is_quiet() && average.at(7).is_none());

    // The triple one starts three averages in (each a bar, over two),
    // and is on a steady rise.
    let tema = indicator(Kind::Tema, &[2.0]).lines(&rising);
    let on = [None, None, None, Some(4.0), Some(5.0), Some(6.0), Some(7.0)];
    assert!(tema.len() == 1 && runs(&tema[0], &on), "{tema:?}");

    // Bands: the mean of the last bars, and two of their standard
    // deviations above and below it, drawn fainter.
    let spread = closing(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]);
    let bands = indicator(Kind::Bands, &[8.0, 2.0]).lines(&spread);
    let [basis, upper, lower] = bands.as_slice() else {
        panic!("bands are three lines: {bands:?}");
    };
    assert!(basis.at(6).is_none() && runs(basis, &[None; 7]));
    assert!(close(basis.at(7).unwrap(), 5.0));
    assert!(close(upper.at(7).unwrap(), 9.0) && close(lower.at(7).unwrap(), 1.0));
    assert_eq!(
        (basis.is_quiet(), upper.is_quiet(), lower.is_quiet()),
        (false, true, true)
    );
    // They move with the bars: flat closes have no spread.
    let narrow = indicator(Kind::Bands, &[2.0, 2.0]).lines(&closing(&[1.0, 3.0, 3.0]));
    assert!(runs(&narrow[0], &[None, Some(2.0), Some(3.0)]));
    assert!(runs(&narrow[1], &[None, Some(4.0), Some(3.0)]));
    assert!(runs(&narrow[2], &[None, Some(0.0), Some(3.0)]));

    // Too few bars compute nothing, and a hidden one has no line.
    let long = indicator(Kind::Ema, &[20.0]).lines(&rising);
    assert!(runs(&long[0], &[None; 7]));
    let three = indicator(Kind::Ema, &[3.0]);
    let hidden = three.clone().with_hidden(true);
    assert!(hidden.lines(&rising).is_empty());

    // All of a chart's: how far they reach over some bars (what the
    // fitted prices make room for), and each one's lines while it is as
    // it was computed.
    let computed = Computed::of(&[three.clone(), hidden.clone()], &rising);
    assert_eq!(computed.bars(), 7);
    assert_eq!(computed.reach(0, 7), Some((2.0, 6.0)));
    assert_eq!(computed.reach(3, 5), Some((3.0, 4.0)));
    assert_eq!(computed.reach(0, 2), None);
    assert!(
        computed
            .lines_of(0, &three)
            .is_some_and(|lines| lines.len() == 1)
    );
    assert!(computed.lines_of(0, &hidden).is_none());
    assert!(computed.lines_of(2, &three).is_none());
    assert!(!computed.is_empty() && !computed.is_blank());
    let unseen = Computed::of(&[hidden], &rising);
    assert!(!unseen.is_empty() && unseen.is_blank());
    assert!(Computed::default().is_empty());
}

#[test]
fn indicator_lines_are_painted_through_their_buckets() {
    use indicators::Kind;
    // `plotted`'s bars, each closing a price above the one before, from
    // 100: an average over three is at 99 and its bar's number.
    let (mut bars, _) = plotted();
    for (bar, candle) in bars.iter_mut().enumerate() {
        candle.close = 100.0 + bar as f64;
    }
    let scene = Scene::build(&bars, &Viewport::showing(50.0), Mode::Candles, 500.0, 116.0);
    let shade = |hue: f32| hsla(hue, 1.0, 0.5, 1.0);
    let ink = indicators::Ink {
        plain: [0.0, 0.1, 0.2, 0.3, 0.4].map(shade),
        quiet: [0.5, 0.6, 0.7, 0.8, 0.9].map(shade),
    };
    let painted = |listed: &[Indicator], scene: &Scene| {
        let computed = Computed::of(listed, &bars);
        Record::of(&indicators::paint(scene, &computed, &ink).finish())
    };
    let starts_at =
        |from: (f32, f32), x: f32, y: f32| (from.0 - x).abs() < 1e-3 && (from.1 - y).abs() < 1e-3;

    // One line through the middle of every bucket the scene has (the
    // fifty under the plot and the one before them), in its color: from
    // bar 49's 148, five points left of the plot, to bar 99's.
    let ema = indicator(Kind::Ema, &[3.0]);
    let line = painted(std::slice::from_ref(&ema), &scene);
    assert_eq!(line.lines.len(), 50);
    assert!(
        starts_at(line.lines[0].from, -5.0, 60.0),
        "{:?}",
        line.lines[0]
    );
    assert!(starts_at(line.lines[1].from, 5.0, 59.0));
    assert!(starts_at(line.lines[49].from, 485.0, 11.0));
    let yellow = ink.plain[LinkColor::Yellow.index()];
    assert!(line.lines.iter().all(|ruled| same(ruled.color, yellow)));
    assert!(line.lines.iter().all(|ruled| !ruled.dashed) && line.texts.is_empty());

    // It starts where its first value is: bar 59's, of sixty bars.
    let late = painted(&[indicator(Kind::Ema, &[60.0])], &scene);
    assert_eq!(late.lines.len(), 40);
    assert!((late.lines[0].from.0 - 95.0).abs() < 1e-3);

    // Bands are their basis and, fainter, the two about it; a hidden
    // indicator paints nothing, and the ones after it still do.
    let bands = Indicator::new(Kind::Bands, std::slice::from_ref(&ema));
    let hidden = ema.clone().with_hidden(true);
    let both = painted(&[hidden, bands], &scene);
    let blue = LinkColor::Blue.index();
    assert_eq!(both.lines.len(), 150);
    assert_eq!(both.ruled(ink.plain[blue]).len(), 50);
    assert_eq!(both.ruled(ink.quiet[blue]).len(), 100);

    // Where bars share a bucket (two each, on a plot a point a bar), the
    // line goes through the newest one's value over the bucket's middle:
    // from the second bucket, the first to hold a bar with one.
    let merged = Scene::build(
        &bars,
        &Viewport::showing(100.0),
        Mode::Candles,
        100.0,
        116.0,
    );
    assert_eq!(merged.bucket, 2);
    let line = painted(std::slice::from_ref(&ema), &merged);
    assert_eq!(line.lines.len(), 48);
    assert!(
        starts_at(line.lines[0].from, 3.0, 106.0),
        "{:?}",
        line.lines[0]
    );
}

#[test]
fn indicators_are_added_hidden_set_and_removed_on_the_plot() {
    let network = Arc::new(StubNetwork::new([(
        format!("POST {FERRIS}/v1/fetchOHLCV"),
        StubRoute::Handler(Arc::new(stub_candles)),
    )]));
    let storage = MemoryStorage::new([]);
    let mut config = EngineConfig::new(Root::of::<Harness>());
    config.viewport = Size::new(px(1280.0), px(800.0));
    config.storage = Some(storage.clone());
    config.network = Some(network);
    config.environment.os = Os::Windows;
    config.environment.idiom = Idiom::Desktop;
    let mut app = Headless::start(config).unwrap();
    // How many lines each canvas ruled when it was last painted (an icon
    // is a canvas too), and how opaque each view is.
    let ruled = Arc::new(Mutex::new(HashMap::<NodeId, usize>::new()));
    let opacities = Arc::new(Mutex::new(HashMap::<NodeId, f32>::new()));
    {
        let (ruled, opacities) = (ruled.clone(), opacities.clone());
        app.observe(move |_, batch| {
            for item in &batch.items {
                match item {
                    MountItem::Canvas { id, drawing } => {
                        let lines = Record::of(drawing).lines.len();
                        ruled.lock().unwrap().insert(*id, lines);
                    }
                    MountItem::ViewStyle { id, style } => {
                        opacities.lock().unwrap().insert(*id, style.opacity);
                    }
                    _ => {}
                }
            }
        });
    }
    // The view whose test id is `id`.
    let node = |screen: &Screen, id: &str| -> Option<NodeId> {
        let nodes = screen.layout_nodes().into_iter();
        nodes
            .filter(|node| node.test_id.as_deref() == Some(id))
            .map(|node| node.id)
            .next()
    };
    // Whether the row's control whose test id ends with `name` shows.
    let control = |screen: &Screen, name: &str| -> bool {
        let control = node(screen, &format!("chart-indicator-0-{name}")).expect("the control");
        opacities.lock().unwrap().get(&control).copied() == Some(1.0)
    };
    // The lines the indicators' layer shows.
    let lines = |screen: &Screen| -> usize {
        let layer = node(screen, "chart-indicator-lines").expect("the indicators' layer");
        ruled.lock().unwrap().get(&layer).copied().unwrap_or(0)
    };
    // What the chart's tab saved as its indicators.
    let saved = move || -> Vec<Value> {
        let entries = storage.entries();
        let workspace = entries.get("lattice.workspace.v1");
        let workspace: Option<Value> =
            workspace.and_then(|workspace| serde_json::from_str(workspace).ok());
        let listed = workspace.and_then(|workspace| member(&workspace, "indicators").cloned());
        listed
            .and_then(|listed| listed.as_array().cloned())
            .unwrap_or_default()
    };
    // The text after `name` on screen: a row's values after its name.
    let after = |screen: &Screen, name: &str| -> Option<String> {
        let texts = screen.texts();
        let at = texts.iter().position(|text| *text == name)?;
        texts.get(at + 1).map(|text| (*text).to_owned())
    };
    let pause = Duration::from_millis(200);

    app.wait_for("BTC/USDT  ·  Bybit").unwrap();
    app.wait_for_gone("Loading historical candles...", WAIT)
        .unwrap();
    app.wait_for("1108.08").unwrap();
    assert!(saved().is_empty() && lines(app.screen()) == 0);

    // The badges float on the plot beside its header, level with it. The
    // timeframe's is 8 points right of the 18 characters the header says
    // (7.2 points each, from 14), on a whole point; the indicators' is 4
    // points right of that one (its two characters, 6 points each, and 6
    // either side), and says its name after its icon (12 points and 4).
    let surface = placed(&app, "chart-plot").expect("the plot");
    let (left, top) = (surface.origin.x.0 - 12.0, surface.origin.y.0 - 12.0);
    let timeframe = placed(&app, "chart-timeframe").expect("the timeframe's badge");
    assert_eq!(
        (timeframe.size.width.0, timeframe.size.height.0),
        (24.0, 16.0)
    );
    assert_eq!(
        (timeframe.origin.x.0 - left, timeframe.origin.y.0 - top),
        (152.0, 14.0)
    );
    let badge = placed(&app, "chart-indicators").expect("the badge");
    assert_eq!((badge.size.width.0, badge.size.height.0), (88.0, 16.0));
    assert_eq!(
        (badge.origin.x.0 - left, badge.origin.y.0 - top),
        (180.0, 14.0)
    );
    assert!(app.shows("INDICATORS"));

    // Its menu lists the kinds; a press on one adds it to the chart as it
    // starts, and closes the menu.
    app.press("Indicators").unwrap();
    app.wait_for("id:chart-indicator-menu").unwrap();
    for kind in indicators::Kind::ALL {
        assert!(app.shows(kind.name()), "{}", kind.name());
    }
    app.press("Add Exponential Moving Average").unwrap();
    app.wait_for_gone("id:chart-indicator-menu", WAIT).unwrap();
    app.wait_until("the average", WAIT, |_| saved().len() == 1)
        .unwrap();
    assert_eq!(
        saved(),
        [json!({ "type": "ema", "length": 20, "color": "yellow" })]
    );
    // It is drawn through the 220 slots that show and the one before
    // them...
    app.wait_until("its line", WAIT, |screen| lines(screen) == 220)
        .unwrap();
    // ...and listed under the header, with its value at the newest bar:
    // within the stub's prices.
    app.wait_for("EMA 20").unwrap();
    let row = placed(&app, "chart-indicator-0").expect("its row");
    assert_eq!((row.origin.x.0 - left, row.origin.y.0 - top), (10.0, 32.0));
    assert_eq!(row.size.height.0, 18.0);
    let newest = after(app.screen(), "EMA 20").expect("its value");
    let value: f64 = newest.parse().expect("a price");
    assert!((1_000.0..1_100.0).contains(&value), "{newest}");
    assert!(app.shows("1108.08"));

    // Its controls show while the pointer is over its row.
    const CONTROLS: [&str; 3] = ["hide", "settings", "remove"];
    assert!(CONTROLS.iter().all(|name| !control(app.screen(), name)));
    app.hover("id:chart-indicator-0").unwrap();
    app.wait_until("the row's controls", WAIT, |screen| {
        CONTROLS.iter().all(|name| control(screen, name))
    })
    .unwrap();

    // With the pointer on the plot, they are gone, the hovered bar's
    // prices show under the row (22 points below where it starts), and
    // the row has the average at that bar, which is not the newest.
    app.hover("id:chart-plot").unwrap();
    app.wait_until("the hovered bar's average", WAIT, |screen| {
        after(screen, "EMA 20").is_some_and(|hovered| hovered != newest)
    })
    .unwrap();
    assert!(CONTROLS.iter().all(|name| !control(app.screen(), name)));
    let hover = placed(&app, "chart-hover").expect("the hovered bar's prices");
    assert_eq!(hover.origin.y.0 - top, 54.0);

    // Hidden, it is off the plot and stays in the legend, without a
    // value; shown again, it is back.
    app.press("Hide EMA 20").unwrap();
    app.wait_until("no line", WAIT, |screen| lines(screen) == 0)
        .unwrap();
    assert_eq!(saved()[0]["hidden"], true);
    assert!(app.shows("EMA 20") && !app.screen().matches("Hide EMA 20").unwrap());
    assert!(!app.shows(&newest));
    // What shows it again stays, with the pointer elsewhere.
    app.wait_until("the control that shows it", WAIT, |screen| {
        control(screen, "hide")
    })
    .unwrap();
    assert!(!control(app.screen(), "remove"));
    app.press("Show EMA 20").unwrap();
    app.wait_until("its line again", WAIT, |screen| lines(screen) == 220)
        .unwrap();
    assert_eq!(saved()[0].get("hidden"), None);

    // Its settings: its length, typed, and its color, picked.
    app.press("EMA 20 settings").unwrap();
    app.wait_for("id:chart-indicator-settings").unwrap();
    app.clear("id:chart-indicator-length").unwrap();
    app.type_text("id:chart-indicator-length", "50\n").unwrap();
    app.wait_for("EMA 50").unwrap();
    assert_eq!(saved()[0]["length"], 50);
    app.press("Color").unwrap();
    app.press("Orange").unwrap();
    app.wait_until("its color", WAIT, |_| saved()[0]["color"] == "orange")
        .unwrap();
    app.press("Close EMA settings").unwrap();
    app.wait_for_gone("id:chart-indicator-settings", WAIT)
        .unwrap();

    // Another takes the first color that is free, and the row under the
    // first. Bands are three lines, and reach past the bars where the
    // stub's prices fall back: the fitted prices make room for them.
    app.press("Indicators").unwrap();
    app.press("Add Bollinger Bands").unwrap();
    app.wait_for("BB 20 2").unwrap();
    assert_eq!(
        saved()[1],
        json!({ "type": "bb", "length": 20, "mult": 2.0, "color": "yellow" })
    );
    app.wait_until("four lines", WAIT, |screen| lines(screen) == 4 * 220)
        .unwrap();
    let second = placed(&app, "chart-indicator-1").expect("the bands' row");
    assert_eq!(second.origin.y.0 - top, 50.0);
    app.wait_for_gone("1108.08", WAIT).unwrap();

    // Removed, they are gone from the plot, the legend and the settings.
    app.press("Remove BB 20 2").unwrap();
    app.wait_until("the average alone", WAIT, |_| saved().len() == 1)
        .unwrap();
    app.wait_for("1108.08").unwrap();
    app.press("Remove EMA 50").unwrap();
    app.wait_until("no indicator", WAIT, |_| saved().is_empty())
        .unwrap();
    app.wait_until("no line", WAIT, |screen| lines(screen) == 0)
        .unwrap();
    app.wait(pause).unwrap();
    assert!(!app.shows("EMA 50") && placed(&app, "chart-indicator-0").is_none());
}

#[test]
fn the_timeframe_badge_charts_the_interval_picked() {
    let network = Arc::new(StubNetwork::new([(
        format!("POST {FERRIS}/v1/fetchOHLCV"),
        StubRoute::Handler(Arc::new(stub_candles)),
    )]));
    let storage = MemoryStorage::new([]);
    let mut config = EngineConfig::new(Root::of::<Harness>());
    config.viewport = Size::new(px(1280.0), px(800.0));
    config.storage = Some(storage.clone());
    config.network = Some(network);
    config.environment.os = Os::Windows;
    config.environment.idiom = Idiom::Desktop;
    let mut app = Headless::start(config).unwrap();
    // What the chart's tab saved as its timeframe.
    let saved = move || -> Option<Value> {
        let entries = storage.entries();
        let workspace: Value = serde_json::from_str(entries.get("lattice.workspace.v1")?).ok()?;
        member(&workspace, "timeframe").cloned()
    };
    // How far right of the plot's view's left edge the view `id` starts.
    let from_left = |app: &Headless, id: &str| -> f32 {
        let surface = placed(app, "chart-plot").expect("the plot");
        let view = placed(app, id).expect("the view");
        view.origin.x.0 - (surface.origin.x.0 - 12.0)
    };

    // The header leaves the interval to the badge beside it: 1m, the
    // default, which saves nothing.
    app.wait_for("BTC/USDT  ·  Bybit").unwrap();
    app.wait_for("label:Timeframe 1m").unwrap();
    assert_eq!(saved(), None);

    // Its menu lists every interval; a press on one charts it and closes
    // the menu.
    app.press("id:chart-timeframe").unwrap();
    app.wait_for("id:chart-timeframe-menu").unwrap();
    for interval in Interval::ALL {
        let row = format!("chart-timeframe-{}", interval.id());
        assert!(placed(&app, &row).is_some(), "{row}");
    }
    app.press("id:chart-timeframe-15m").unwrap();
    app.wait_for_gone("id:chart-timeframe-menu", WAIT).unwrap();
    app.wait_for("label:Timeframe 15m").unwrap();
    app.wait_until("the timeframe saved", WAIT, |_| {
        saved() == Some(json!("15m"))
    })
    .unwrap();
    // The badge is as wide as what it says, a character more, and the
    // indicators' moved over by it.
    let badge = placed(&app, "chart-timeframe").expect("the badge");
    assert_eq!(badge.size.width.0, 30.0);
    assert_eq!(from_left(&app, "chart-timeframe"), 152.0);
    assert_eq!(from_left(&app, "chart-indicators"), 186.0);

    // Escape closes the menu and changes nothing.
    app.press("id:chart-timeframe").unwrap();
    app.wait_for("id:chart-timeframe-menu").unwrap();
    app.press_key("Escape").unwrap();
    app.wait_for_gone("id:chart-timeframe-menu", WAIT).unwrap();
    assert_eq!(saved(), Some(json!("15m")));

    // Ticks are one of them, and the default one saves nothing again.
    app.press("id:chart-timeframe").unwrap();
    app.press("id:chart-timeframe-tick").unwrap();
    app.wait_for("label:Timeframe Tick").unwrap();
    app.wait_until("ticks saved", WAIT, |_| saved() == Some(json!("tick")))
        .unwrap();
    app.press("id:chart-timeframe").unwrap();
    app.press("id:chart-timeframe-1m").unwrap();
    app.wait_for("label:Timeframe 1m").unwrap();
    app.wait_until("nothing saved", WAIT, |_| saved().is_none())
        .unwrap();
}
