use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use nsh_core::{
    MountItem,
    element::Root,
    platform::{Idiom, Os},
    prelude::*,
    shader::{ShaderLayer, UniformValue},
};
use nsh_engine::{
    EngineConfig, HttpBody, HttpRequest,
    shader::{Frame, PixelFormat, Renderer, ShaderView},
};
use nsh_testing::{Headless, MemoryStorage, StubNetwork, StubReply, StubResponse, StubRoute, WAIT};
use serde_json::{Value, json};

use super::{
    scene::{Mode, Scene},
    settings::{ChartSettings, Interval},
    shader::{self, Palette},
    viewport::{Prices, Viewport},
};
use crate::{
    data::{
        DataServices,
        ferris::{Candle, FerrisConfig, MarketSelection, Timeframe},
    },
    layout,
    shell::clock::Clock,
    storage::StoredWorkspace,
    widgets::{WidgetKind, picker::Picker},
};

fn candle(time: f64, open: f64, high: f64, low: f64, close: f64) -> Candle {
    Candle {
        time,
        open,
        high,
        low,
        close,
        volume: None,
    }
}

/// `count` one-minute bars, each opening at its number and closing one
/// above it.
fn series(count: usize) -> Vec<Candle> {
    (0..count)
        .map(|at| {
            let open = at as f64;
            candle(open * 60.0, open, open + 2.0, open - 1.0, open + 1.0)
        })
        .collect()
}

#[test]
fn the_viewport_shows_the_newest_slots_and_pans_within_the_series() {
    let view = Viewport::showing(220.0);
    assert_eq!(view.window(1_600), (1_380.0, 1_600.0));
    // The oldest bar stops at the left edge, the newest one too.
    assert_eq!(view.panned(5_000.0, 1_600).pan, 1_380.0);
    assert_eq!(view.panned(-5_000.0, 1_600).pan, -219.0);
    // Fewer bars than slots: nothing older to look at.
    assert_eq!(view.panned(10.0, 100).pan, 0.0);
}

#[test]
fn zooming_keeps_the_slot_under_the_pointer() {
    let view = Viewport::showing(200.0).panned(500.0, 2_000);
    let (left, _) = view.window(2_000);
    let zoomed = view.zoomed(0.5, 0.25, 2_000);
    assert_eq!(zoomed.visible, 100.0);
    let (zoomed_left, _) = zoomed.window(2_000);
    assert_eq!(zoomed_left + 0.25 * 100.0, left + 0.25 * 200.0);
    // At the newest bar, zooming out opens no empty slots after it.
    assert_eq!(Viewport::showing(200.0).zoomed(2.0, 0.5, 2_000).pan, 0.0);
    // Where there are some, the newest bar keeps its place across the plot.
    let ahead = Viewport::showing(200.0)
        .panned(-50.0, 2_000)
        .zoomed(2.0, 0.5, 2_000);
    assert_eq!((ahead.visible, ahead.pan), (400.0, -100.0));
    // The old chart's limits.
    assert_eq!(view.zoomed(0.001, 0.5, 2_000).visible, 24.0);
    assert_eq!(view.zoomed(1e9, 0.5, 2_000).visible, 25_000.0);
}

#[test]
fn the_time_axis_drag_and_new_bars_move_the_viewport() {
    let origin = Viewport::showing(100.0).panned(40.0, 1_000);
    let wider = Viewport::stretched(origin, 220.0, 1_000);
    assert!((wider.visible - 100.0 * std::f64::consts::E).abs() < 1e-9);
    assert_eq!(wider.pan, 40.0);
    // A plot at the newest bar follows it; one looking back stays put.
    assert_eq!(Viewport::showing(100.0).held(3, 1_003).pan, 0.0);
    assert_eq!(origin.held(3, 1_003).pan, 43.0);
}

#[test]
fn prices_fit_scale_and_move_as_the_old_chart_did() {
    assert_eq!(
        Prices::fitting(100.0, 200.0),
        Prices {
            low: 92.0,
            high: 208.0
        }
    );
    // A flat range is opened by 1% of its price first.
    let flat = Prices::fitting(50.0, 50.0);
    assert!((flat.low - 49.42).abs() < 1e-9 && (flat.high - 50.58).abs() < 1e-9);

    let range = Prices {
        low: 100.0,
        high: 200.0,
    };
    assert_eq!(range.fraction(175.0), 0.25);
    assert_eq!(range.price_at(0.25), 175.0);
    // Dragging half the plot's height down shows prices half a span up.
    assert_eq!(
        range.dragged(50.0, 100.0),
        Prices {
            low: 150.0,
            high: 250.0
        }
    );
    // The wheel keeps the price under the pointer where it is.
    let zoomed = range.zoomed(0.5, 0.25, 1.0);
    assert_eq!(zoomed.span(), 50.0);
    assert_eq!(zoomed.price_at(0.25), 175.0);
    // Scaling stops at the least span.
    assert_eq!(range.scaled(-10_000.0, 2.0).span(), 2.0);
    assert_eq!(range.scaled(0.0, 2.0), range);
}

#[test]
fn narrow_slots_merge_into_buckets_counted_from_the_first_bar() {
    let bars = series(2_000);
    // 1000 slots on 500 points: two points a candle needs four bars each.
    let view = Viewport::showing(1_000.0);
    let scene = Scene::build(&bars, &view, Mode::Candles, 500.0, 300.0);
    assert_eq!(scene.bucket, 4);
    assert_eq!((scene.left, scene.right), (1_000.0, 2_000.0));
    // The buckets under the plot and one before it.
    assert_eq!((scene.first_bucket, scene.buckets.len()), (249, 251));
    let merged = scene.buckets[1];
    assert_eq!(merged, candle(60_000.0, 1_000.0, 1_005.0, 999.0, 1_004.0));
    // Half a bucket's pan leaves every bucket its bars.
    let panned = Scene::build(&bars, &view.panned(2.0, 2_000), Mode::Candles, 500.0, 300.0);
    assert_eq!(panned.first_bucket, 248);
    assert_eq!(panned.buckets[1..3], scene.buckets[..2]);
    // The prices fit the buckets that show.
    assert_eq!(scene.fit, Prices::fitting(999.0, 2_001.0));
    assert_eq!(scene.prices, scene.fit);
}

#[test]
fn tiles_cut_the_plot_and_carry_their_buckets() {
    let bars = series(2_000);
    let view = Viewport {
        prices: Some(Prices {
            low: 0.0,
            high: 2_000.0,
        }),
        ..Viewport::showing(1_000.0)
    };
    let scene = Scene::build(&bars, &view, Mode::Candles, 500.0, 300.0);
    let [left, right] = scene.tiles.as_slice() else {
        panic!("500 points are two tiles: {:?}", scene.tiles.len());
    };
    assert_eq!((left.left, left.width), (0.0, 250.0));
    assert_eq!((right.left, right.width), (250.0, 250.0));
    // Each has the buckets under it, one before and the two after its
    // edge (the series ends at the right tile's).
    assert_eq!((left.first, left.per, left.count), (1.0, 125.0, 128));
    assert_eq!((right.first, right.per, right.count), (1.0, 125.0, 126));
    assert_eq!(left.values.len(), left.count * 4);
    // Bucket 250 (bars 1000 to 1003) is the left tile's second: its open,
    // high, low and close as fractions down the plot.
    assert_eq!(left.values[4..8], [0.5, 0.4975, 0.5005, 0.498]);
    // The right tile starts one bucket before its edge, where the left
    // one's last but one is.
    assert_eq!(right.values[..4], left.values[125 * 4..126 * 4]);

    // A line has a close per bucket, a point apart at its narrowest.
    let line = Scene::build(&bars, &view, Mode::Line, 500.0, 300.0);
    assert_eq!(line.bucket, 2);
    assert_eq!(line.tiles[0].values.len(), line.tiles[0].count);
    assert_eq!(line.tiles[0].count, 253);
}

#[test]
fn the_crosshair_and_the_time_axis_find_their_bars() {
    let bars = series(100);
    // Fifty slots: the newest forty bars, then ten empty ones.
    let view = Viewport::showing(50.0).panned(-10.0, 100);
    let scene = Scene::build(&bars, &view, Mode::Candles, 500.0, 300.0);
    assert_eq!(scene.bucket, 1);
    // Bar 60 fills the first ten points.
    assert_eq!(scene.bucket_near(4.0), Some((5.0, bars[60])));
    assert_eq!(scene.bucket_near(399.0), Some((395.0, bars[99])));
    // Past the newest bar, the newest one.
    assert_eq!(scene.bucket_near(480.0), Some((395.0, bars[99])));
    let marks = scene.time_marks(3);
    assert_eq!(
        marks,
        [
            (5.0, bars[60].time),
            (205.0, bars[80].time),
            (395.0, bars[99].time)
        ]
    );
    assert_eq!(Scene::empty(Mode::Candles).bucket_near(0.0), None);
    assert!(Scene::empty(Mode::Line).time_marks(7).is_empty());
}

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

/// Opaque colors that tell the layers apart.
fn palette() -> Palette {
    Palette {
        rising: [0.0, 1.0, 0.0, 1.0].into(),
        falling: [1.0, 0.0, 0.0, 1.0].into(),
        line: [0.0, 0.0, 1.0, 1.0].into(),
        grid: [1.0, 1.0, 1.0, 0.4].into(),
    }
}

/// The shader's layer for the one tile of `scene`.
fn layer(scene: &Scene, grid: bool) -> ShaderLayer {
    let program = shader::program().expect("the chart's shader compiles");
    let uniforms = shader::uniforms(scene, &scene.tiles[0], grid, &palette());
    let mut warnings = Vec::new();
    let layer = ShaderLayer::new(program, &[("d".into(), uniforms)], &mut warnings);
    assert_eq!(warnings, Vec::<String>::new());
    layer
}

/// Four candles on a 96 by 64 point plot whose prices are its pixels from
/// the bottom: 24 points a slot.
fn four_candles() -> Scene {
    let bars = [
        candle(0.0, 16.0, 56.0, 8.0, 48.0),
        candle(60.0, 40.0, 44.0, 12.0, 20.0),
        candle(120.0, 20.0, 22.0, 18.0, 21.0),
        candle(180.0, 21.0, 34.0, 20.0, 30.0),
    ];
    let view = Viewport {
        prices: Some(Prices {
            low: 0.0,
            high: 64.0,
        }),
        ..Viewport::showing(4.0)
    };
    Scene::build(&bars, &view, Mode::Candles, 96.0, 64.0)
}

#[test]
fn the_shader_compiles_and_takes_a_tile_in_one_array() {
    let program = shader::program().expect("the chart's shader compiles");
    assert_eq!(program.uniform_size(), 4_096);
    assert!(
        !program.animated(),
        "it draws only when its uniforms change"
    );
    assert!(program.inputs().is_empty());

    let scene = four_candles();
    let layer = layer(&scene, true);
    let word = |at: usize| f32::from_bits(layer.uniforms[at]);
    // The view: the tile starts at its first bucket, four across, four
    // candles.
    assert_eq!([word(0), word(1), word(2), word(3)], [0.0, 4.0, 4.0, 0.0]);
    // The plot: the tile's left edge, the plot's width, the grid's
    // columns and rows.
    assert_eq!([word(4), word(5), word(6), word(7)], [0.0, 96.0, 7.0, 6.0]);
    // The falling candles' color.
    assert_eq!(
        [word(12), word(13), word(14), word(15)],
        [1.0, 0.0, 0.0, 1.0]
    );
    // The first candle after the head: open, high, low, close down the plot.
    assert_eq!(
        [word(24), word(25), word(26), word(27)],
        [0.75, 0.125, 0.875, 0.25]
    );
    // Whatever a tile holds fits the array.
    let full = Scene::build(
        &series(5_000),
        &Viewport::showing(5_000.0),
        Mode::Candles,
        480.0,
        64.0,
    );
    assert!(matches!(
        shader::uniforms(&full, &full.tiles[0], true, &palette()),
        UniformValue::List(slots) if slots.len() <= 256
    ));
}

/// `layer` drawn by NSH's renderer (wgpu) at `width` by `height` pixels;
/// none on a machine without a GPU.
fn drawn(layer: ShaderLayer, width: f32, height: f32) -> Option<Frame> {
    let renderer = Renderer::new(PixelFormat::Rgba)?;
    let mut view = ShaderView::default();
    view.set_layer(Some(Arc::new(layer)));
    view.set_size(Size::new(px(width), px(height)), 1.0);
    let started = Instant::now();
    while !view.render(&renderer, Instant::now()) {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the GPU returned no frame"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    Some(view.frame().clone())
}

/// The pixel at (`x`, `y`) of `frame`: premultiplied red, green, blue and
/// alpha.
fn pixel(frame: &Frame, x: u32, y: u32) -> [u8; 4] {
    let at = (y * frame.width + x) as usize * 4;
    frame.tight()[at..at + 4]
        .try_into()
        .expect("a pixel is four bytes")
}

const CLEAR: [u8; 4] = [0, 0, 0, 0];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

#[test]
fn the_gpu_draws_candles_where_the_scene_puts_them() {
    let scene = four_candles();
    let Some(frame) = drawn(layer(&scene, false), 96.0, 64.0) else {
        eprintln!("no GPU here: the chart's pixels were not checked");
        return;
    };
    assert_eq!((frame.width, frame.height), (96, 64));
    // The first candle rose from 16 to 48: its body is rows 16 to 47,
    // sixteen pixels about column 12; its wick rows 8 to 55 of column 12.
    assert_eq!(pixel(&frame, 12, 30), GREEN);
    assert_eq!(pixel(&frame, 5, 16), GREEN);
    assert_eq!(pixel(&frame, 19, 47), GREEN);
    assert_eq!(pixel(&frame, 4, 30), CLEAR);
    assert_eq!(pixel(&frame, 20, 30), GREEN);
    assert_eq!(pixel(&frame, 21, 30), CLEAR);
    assert_eq!(pixel(&frame, 12, 15), GREEN);
    assert_eq!(pixel(&frame, 12, 8), GREEN);
    assert_eq!(pixel(&frame, 12, 7), CLEAR);
    assert_eq!(pixel(&frame, 12, 55), GREEN);
    assert_eq!(pixel(&frame, 12, 56), CLEAR);
    assert_eq!(pixel(&frame, 11, 10), CLEAR);
    // The second fell from 40 to 20: rows 24 to 43 about column 36.
    assert_eq!(pixel(&frame, 36, 30), RED);
    assert_eq!(pixel(&frame, 36, 20), RED);
    assert_eq!(pixel(&frame, 36, 19), CLEAR);
    assert_eq!(pixel(&frame, 44, 43), RED);
    assert_eq!(pixel(&frame, 45, 43), CLEAR);
    assert_eq!(pixel(&frame, 24, 30), CLEAR);
    // Nothing else: the plot is clear where no candle is.
    assert_eq!(pixel(&frame, 50, 34), CLEAR);

    // The grid: white hairlines at 40% every seventh of the width and
    // sixth of the height, under the candles.
    let gridded = drawn(layer(&scene, true), 96.0, 64.0).expect("the GPU answered before");
    let grid = [102, 102, 102, 102];
    assert_eq!(pixel(&gridded, 13, 2), grid);
    assert_eq!(pixel(&gridded, 2, 10), grid);
    assert_eq!(pixel(&gridded, 2, 2), CLEAR);
    assert_eq!(pixel(&gridded, 13, 30), GREEN);
}

#[test]
fn the_gpu_draws_a_line_through_the_closes() {
    // Three ticks 48 points apart, through pixel centers: level along row
    // 16, then down to row 48.
    let tick = |time: f64, price: f64| candle(time, price, price, price, price);
    let bars = [tick(0.0, 47.5), tick(1.0, 47.5), tick(2.0, 15.5)];
    let view = Viewport {
        prices: Some(Prices {
            low: 0.0,
            high: 64.0,
        }),
        ..Viewport::showing(3.0)
    };
    let scene = Scene::build(&bars, &view, Mode::Line, 144.0, 64.0);
    let Some(frame) = drawn(layer(&scene, false), 144.0, 64.0) else {
        eprintln!("no GPU here: the chart's pixels were not checked");
        return;
    };
    // From (24, 16.5) to (72, 16.5), a point and a half thick.
    assert_eq!(pixel(&frame, 48, 16), BLUE);
    assert_eq!(pixel(&frame, 48, 14), CLEAR);
    assert_eq!(pixel(&frame, 48, 18), CLEAR);
    // On to (120, 48.5), through the middle of pixel (73, 17).
    assert_eq!(pixel(&frame, 73, 17), BLUE);
    assert_eq!(pixel(&frame, 100, 20), CLEAR);
    // Nothing before the first tick or under the line.
    assert_eq!(pixel(&frame, 10, 16), CLEAR);
    assert_eq!(pixel(&frame, 0, 48), CLEAR);
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
