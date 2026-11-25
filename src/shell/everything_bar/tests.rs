use std::{
    rc::Rc,
    sync::{Arc, Mutex},
};

use super::*;
use nsh_core::{
    element::Root,
    platform::{Idiom, Os},
};
use nsh_engine::{EngineConfig, HttpBody, HttpRequest};
use nsh_testing::{Headless, MemoryStorage, StubNetwork, StubReply, StubResponse, StubRoute, WAIT};
use serde_json::{Value, json};

use crate::{
    data::{
        DataConfig, DataServices,
        ferris::FerrisConfig,
        hyperliquid::{HyperliquidConfig, signing},
        wallet::{Event, Reach, WalletConfig, testing::StandIn},
    },
    widgets::WidgetKind,
};

const CODE: &str = "id:layout-code";

/// The bar over a two-page workspace, with a line reporting the committed
/// workspace so tests can observe what the Layout Manager changed.
#[component]
fn Harness() -> impl IntoElement {
    let mut saved = StoredWorkspace::default();
    layout::normalize(&mut saved);
    layout::add_page(&mut saved);
    let workspace = signal(saved);
    view()
        .size_full()
        .flex_col()
        .child(bar(
            workspace,
            ActiveTheme::restore(),
            Clock::restore(),
            crate::widgets::picker::Picker::new(),
        ))
        .child(text(move || {
            let state = workspace.get();
            format!(
                "Pages {} active {}",
                state.pages.len(),
                state.active_page_id
            )
        }))
}

fn start() -> Headless {
    start_with(MemoryStorage::new([]))
}

fn start_with(storage: Arc<MemoryStorage>) -> Headless {
    let mut app = Headless::start(EngineConfig {
        viewport: Size::new(px(1280.0), px(800.0)),
        storage: Some(storage),
        ..EngineConfig::new(nsh_testing::windowed_root::<Harness>())
    })
    .unwrap();
    app.wait_for("Pages 2 active 2").unwrap();
    app
}

fn open(app: &mut Headless) {
    app.press("Layout manager").unwrap();
    app.wait_for("id:layout-manager").unwrap();
}

/// The bar on its own, on a desktop of `os`.
fn start_on(os: Os) -> Headless {
    let mut config = EngineConfig::new(nsh_testing::windowed_root::<Harness>());
    config.viewport = Size::new(px(1280.0), px(800.0));
    config.storage = Some(MemoryStorage::new([]));
    config.environment.os = os;
    config.environment.idiom = Idiom::Desktop;
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Pages 2 active 2").unwrap();
    app
}

#[test]
fn windows_gets_its_window_buttons_from_the_bar_and_other_platforms_keep_their_own() {
    let buttons = ["Minimize window", "Maximize window", "Close window"];
    let shown = |os: Os| {
        let app = start_on(os);
        buttons.map(|button| app.screen().matches(&format!("label:{button}")).unwrap())
    };
    assert_eq!(shown(Os::Windows), [true; 3]);
    for os in [Os::MacOs, Os::Linux, Os::Web] {
        assert_eq!(shown(os), [false; 3], "{os:?}");
    }
}

#[test]
fn close_button_layout_button_and_escape_close_the_manager() {
    let mut app = start();
    open(&mut app);
    app.press("Close Layout Manager").unwrap();
    app.wait_for_gone("id:layout-manager", WAIT).unwrap();

    open(&mut app);
    app.press("Layout manager").unwrap();
    app.wait_for_gone("id:layout-manager", WAIT).unwrap();

    open(&mut app);
    app.press_key("Escape").unwrap();
    app.wait_for_gone("id:layout-manager", WAIT).unwrap();
}

#[test]
fn clear_asks_first_then_deletes_and_finally_empties_the_last_page() {
    let mut app = start();
    open(&mut app);
    let question = "Delete layout 2 and its widgets? This cannot be undone.";

    app.press("Clear layout 2").unwrap();
    app.wait_for(question).unwrap();
    app.press("Cancel").unwrap();
    app.wait_for_gone(question, WAIT).unwrap();
    assert!(app.shows("Pages 2 active 2"));

    app.press("Clear layout 2").unwrap();
    app.press("Delete layout").unwrap();
    app.wait_for("Pages 1 active 1").unwrap();
    app.wait_for("Layout 2 cleared.").unwrap();

    app.press("Clear layout 1").unwrap();
    app.wait_for("Remove every widget from layout 1? This cannot be undone.")
        .unwrap();
    app.press("Delete layout").unwrap();
    app.wait_for("Layout 1 cleared.").unwrap();
    assert!(app.shows("Pages 1 active 1"));
}

#[test]
fn generated_codes_import_after_confirmation() {
    let mut app = start();
    open(&mut app);
    app.press("Generate layout code").unwrap();
    app.wait_for("Layout code generated and copied.").unwrap();
    let code = app.screen().input_text(CODE).unwrap().unwrap().to_owned();
    assert!(code.starts_with("LATTICE1."));
    assert_eq!(app.screen().clipboard(), Some(code.as_str()));

    app.press("Add layout page").unwrap();
    app.wait_for("Pages 3 active 3").unwrap();
    app.press("Import layout").unwrap();
    app.wait_for("Replace layouts").unwrap();
    assert!(
        app.shows("Pages 3 active 3"),
        "import must wait for confirmation"
    );
    app.press("Replace layouts").unwrap();
    app.wait_for("Pages 2 active 2").unwrap();
    app.wait_for("Layout imported.").unwrap();
}

#[test]
fn the_code_field_is_editable_and_rejects_invalid_codes() {
    let mut app = start();
    open(&mut app);
    assert!(!app.screen().enabled("Import layout").unwrap());

    // Return in the field imports, like the button.
    app.type_text(CODE, "nope\n").unwrap();
    assert_eq!(app.screen().input_text(CODE).unwrap(), Some("nope"));
    app.wait_for("Code must start with LATTICE1.").unwrap();
    assert!(app.screen().enabled("Import layout").unwrap());

    // Editing again clears the error.
    app.type_text(CODE, "!").unwrap();
    app.wait_for_gone("Code must start with LATTICE1.", WAIT)
        .unwrap();

    app.clear(CODE).unwrap();
    app.next_change(WAIT).ok();
    assert!(!app.screen().enabled("Import layout").unwrap());

    // The field keeps its text while the manager is closed.
    app.type_text(CODE, "LATTICE1.00").unwrap();
    app.press("Close Layout Manager").unwrap();
    app.wait_for_gone("id:layout-manager", WAIT).unwrap();
    open(&mut app);
    assert_eq!(app.screen().input_text(CODE).unwrap(), Some("LATTICE1.00"));
}

/// The color the bar's "Layout" button is drawn in: the theme's muted text.
fn muted(app: &Headless) -> nsh_core::Color {
    app.screen().span_style("text:Layout").unwrap().color
}

#[test]
fn settings_switch_the_theme_in_place_and_remember_it() {
    let storage = MemoryStorage::new([]);
    let mut app = start_with(storage.clone());
    app.press("Application settings").unwrap();
    app.wait_for("id:settings").unwrap();
    assert!(app.shows("Dark"));
    let dark = muted(&app);

    // The arrows step along the list and wrap at either end; what is open
    // repaints in place, the panel included.
    let last = theme::all().last().unwrap().name.as_str();
    app.press("Next theme").unwrap();
    app.wait_for("Light").unwrap();
    assert_ne!(muted(&app), dark);
    assert!(app.screen().matches("id:settings").unwrap());
    app.press("Previous theme").unwrap();
    app.wait_for("Dark").unwrap();
    assert_eq!(muted(&app), dark);
    app.press("Previous theme").unwrap();
    app.wait_for(last).unwrap();
    app.press("Next theme").unwrap();
    app.wait_for("Dark").unwrap();

    // The dropdown chooses directly. Escape closes its list before the panel.
    app.press("role:combobox[Theme]").unwrap();
    app.press("role:option[Nord]").unwrap();
    app.wait_for_gone("role:option[Nord]", WAIT).unwrap();
    app.wait_for("Nord").unwrap();
    let nord = muted(&app);
    assert_ne!(nord, dark);
    app.press("role:combobox[Theme]").unwrap();
    app.wait_for("role:option[Nord]").unwrap();
    app.press_key("Escape").unwrap();
    app.wait_for_gone("role:option[Nord]", WAIT).unwrap();
    assert!(app.screen().matches("id:settings").unwrap());
    app.press_key("Escape").unwrap();
    app.wait_for_gone("id:settings", WAIT).unwrap();

    // The choice is saved, and the next launch opens in it.
    drop(app);
    let app = start_with(storage);
    assert_eq!(muted(&app), nord);
}

/// A Ferris no test reaches: its HTTP is the test's stub network, and
/// nothing listens where its socket would connect.
const FERRIS: &str = "https://ferris.test";

/// The shell as `shell()` makes it, over the stubbed Ferris, with one
/// chart in the workspace.
#[component]
fn Shell() -> impl IntoElement {
    stubbed_shell(1)
}

/// [`Shell`] with a second chart, of the same market.
#[component]
fn TwoCharts() -> impl IntoElement {
    stubbed_shell(2)
}

fn stubbed_shell(charts: usize) -> View {
    let services = DataServices::provide_with(FerrisConfig {
        http_base: FERRIS.to_owned(),
        ws_url: "ws://127.0.0.1:9/v1/ws".to_owned(),
        ..FerrisConfig::production()
    });
    let clock = Clock::restore();
    provide_context(clock);
    let mut saved = StoredWorkspace::default();
    layout::normalize(&mut saved);
    for _ in 0..charts {
        layout::spawn(&mut saved, WidgetKind::Chart);
    }
    let workspace = signal(saved);
    let picker = crate::widgets::picker::Picker::new();
    services
        .sessions
        .follow(move || layout::widget_ids(&workspace.get()));
    view()
        .size_full()
        .flex_col()
        .child(bar(workspace, ActiveTheme::restore(), clock, picker))
        .child(layout::workspace(workspace, picker))
        .child(picker.view(workspace))
}

/// What a stubbed exchange supports: catalogs, candles, and statistics of
/// its perpetuals' volume and mark price.
fn stub_exchange(id: &str) -> Value {
    let every = json!({ "state": "supported", "supportedProducts": ["perp", "spot", "future"] });
    json!({
        "exchange": id,
        "features": {
            "fetchMarkets": every,
            "fetchOhlcv": every,
            "realtime": { "ohlcv": every, "trades": every },
        },
        "marketStats": {
            "state": "supported",
            "scope": { "exchange": id, "params": {} },
            "fields": { "perp": {
                "volume24h": { "state": "supported" },
                "markPrice": { "state": "supported" },
            } },
            "pollIntervalMs": 5000,
            "staleAfterMs": 60000,
        },
    })
}

/// A stubbed catalog market, and the id its statistics share.
fn stub_market(exchange: &str, base: &str, quote: &str, kind: &str) -> Value {
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

/// The stubbed markets: each exchange's, with each perpetual's 24h quote
/// volume. Hyperliquid's BTC/USDC trades the most.
fn stub_markets(exchange: &str) -> Vec<(Value, Option<f64>)> {
    let market = |base, quote, kind, volume| (stub_market(exchange, base, quote, kind), volume);
    match exchange {
        "bybit" => vec![
            market("BTC", "USDT", "perp", Some(9.0e8)),
            market("BTC", "USDC", "perp", Some(1.0e8)),
            market("ETH", "USDT", "perp", Some(5.0e8)),
            market("BTC", "USDC", "spot", None),
        ],
        "hyperliquid" => vec![
            market("BTC", "USDC", "perp", Some(3.8e9)),
            market("ETH", "USDC", "perp", Some(2.0e9)),
            market("BTC", "USDC", "spot", None),
        ],
        _ => Vec::new(),
    }
}

/// Answers the stubbed Ferris's `POST`s: catalogs, statistics, and no
/// candles.
fn stub_ferris(request: HttpRequest, reply: StubReply) {
    let body: Value = match &request.body {
        HttpBody::Bytes(bytes) => serde_json::from_slice(bytes).unwrap_or_default(),
        _ => Value::Null,
    };
    let exchange = body["exchange"].as_str().unwrap_or_default();
    let answer = if request.url.ends_with("/v1/fetchMarkets") {
        let markets: Vec<Value> = stub_markets(exchange)
            .into_iter()
            .map(|(market, _)| market)
            .collect();
        json!(markets)
    } else if request.url.ends_with("/v1/fetchMarketStats") {
        // Only the linear perpetuals have statistics here.
        let linear = body["params"]["category"].is_null();
        let markets: Vec<Value> = stub_markets(exchange)
            .into_iter()
            .filter_map(|(mut market, volume)| {
                let volume = volume.filter(|_| linear)?;
                let quote = market["quote"].clone();
                let base = market["base"].clone();
                market["fields"] = json!({
                    "volume24h": {
                        "state": "available",
                        "value": { "baseVolume": null, "quoteVolume": volume },
                    },
                    "markPrice": {
                        "state": "available",
                        "value": { "amount": "100.0", "baseAsset": base, "quoteAsset": quote },
                    },
                });
                Some(market)
            })
            .collect();
        json!({
            "timestamp": 0,
            "coverage": { "returnedMarkets": markets.len(), "enumerationComplete": true },
            "markets": markets,
        })
    } else {
        json!([])
    };
    let mut response = StubResponse::default();
    response.status = 200;
    response.body = serde_json::to_vec(&answer).expect("the answer is JSON");
    reply.send(response);
}

fn start_shell() -> Headless {
    start_root(nsh_testing::windowed_root::<Shell>())
}

fn start_root(root: Root) -> Headless {
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
    let mut config = EngineConfig::new(root);
    config.viewport = Size::new(px(1280.0), px(800.0));
    config.storage = Some(MemoryStorage::new([]));
    config.network = Some(network);
    config.environment.os = Os::Windows;
    config.environment.idiom = Idiom::Desktop;
    let mut app = Headless::start(config).unwrap();
    app.wait_for("BTC/USDT  ·  Bybit").unwrap();
    app
}

/// Checks the first results in order, by walking the keyboard's row down
/// them: it starts on the first, and each is `selected` while it has it.
fn assert_results(app: &mut Headless, expected: &[&str]) {
    for (index, label) in expected.iter().enumerate() {
        if index > 0 {
            app.press_key("down").unwrap();
        }
        assert!(
            app.screen().selected(label).unwrap(),
            "result {index} should be {label}"
        );
    }
}

/// The connection status: the bar's indicator and the panel it opens.
const INDICATOR: &str = "id:connection-status";
const CONNECTIONS: &str = "id:connection-status-panel";

/// Whether the connection panel shows the text `text`.
fn panel_shows(app: &Headless, text: &str) -> bool {
    app.screen()
        .matches(&format!("{CONNECTIONS} >> text:{text}"))
        .unwrap()
}

#[test]
fn the_connection_indicator_opens_nothing_without_data_services() {
    // The bar on its own: nothing above it has a source to report.
    let app = start();
    assert!(!app.screen().enabled(INDICATOR).unwrap());
    assert!(
        app.screen()
            .matches("label:Connection status: neutral")
            .unwrap()
    );
    assert!(!app.screen().matches(CONNECTIONS).unwrap());
}

#[test]
fn the_connection_panel_lists_what_the_widgets_stream_and_poll() {
    let mut app = start_root(nsh_testing::windowed_root::<TwoCharts>());

    // The indicator has the sources' health: the charts ask for a stream,
    // and nothing listens where the socket connects.
    app.wait_until("the indicator showing trouble", WAIT, |screen| {
        ["warning", "error"]
            .iter()
            .any(|word| screen.matches(&format!("label:Connection status: {word}")) == Ok(true))
    })
    .unwrap();

    // A press opens the panel, folded: the socket the charts stream on and
    // the requests the market directory polls.
    app.press(INDICATOR).unwrap();
    app.wait_for(CONNECTIONS).unwrap();
    assert!(panel_shows(&app, "2 sources"));
    for heading in ["Ferris realtime", "WS", "Ferris requests", "HTTP"] {
        assert!(panel_shows(&app, heading), "{heading} should be a heading");
    }
    assert!(!panel_shows(&app, "BTC/USDT:USDT"));

    // The socket's topics: the candles both charts share, one topic, which
    // waits for a socket nothing answers here.
    app.press("id:connection-source-ferris.socket").unwrap();
    app.wait_for(&format!("{CONNECTIONS} >> text:BTC/USDT:USDT"))
        .unwrap();
    let topics = app
        .screen()
        .count(&format!("{CONNECTIONS} >> text:BTC/USDT:USDT"))
        .unwrap();
    assert_eq!(topics, 1);
    assert!(panel_shows(&app, "Candles · linear · 1m"));
    assert!(panel_shows(&app, "Chart ×2"));
    assert!(panel_shows(&app, "Waiting for source"));
    assert!(panel_shows(&app, "—"), "no candle has streamed");

    // The requests: the capabilities and every catalog, which the market
    // directory keeps for the search and the pickers.
    app.press("id:connection-source-ferris.queries").unwrap();
    app.wait_for(&format!("{CONNECTIONS} >> text:Capabilities"))
        .unwrap();
    assert!(panel_shows(&app, "What each exchange supports · every 5m"));
    assert!(panel_shows(&app, "Market catalog · every 1m"));
    let polled = app
        .screen()
        .count(&format!("{CONNECTIONS} >> text:Market directory"))
        .unwrap();
    assert!(polled >= 3, "the capabilities and two exchanges' catalogs");
    assert!(panel_shows(&app, "Up to date"));

    // Folding a source takes its rows away again.
    app.press("id:connection-source-ferris.socket").unwrap();
    app.wait_for_gone(&format!("{CONNECTIONS} >> text:BTC/USDT:USDT"), WAIT)
        .unwrap();

    // The report names what is wrong and who it is wrong for.
    app.press("Copy report").unwrap();
    app.wait_for("· Report copied").unwrap();
    let report = app.screen().clipboard().unwrap();
    assert!(report.starts_with("Lattice connection issues\nCaptured: "));
    assert!(
        report.contains("\nBybit · BTC/USDT:USDT · Waiting for source\n  Components: Chart ×2\n")
    );

    // Its close button, the indicator and Escape close it.
    app.press("Close connection status").unwrap();
    app.wait_for_gone(CONNECTIONS, WAIT).unwrap();
    app.press(INDICATOR).unwrap();
    app.wait_for(CONNECTIONS).unwrap();
    assert!(!panel_shows(&app, "Capabilities"), "it opens folded");
    app.press(INDICATOR).unwrap();
    app.wait_for_gone(CONNECTIONS, WAIT).unwrap();
    app.press(INDICATOR).unwrap();
    app.wait_for(CONNECTIONS).unwrap();
    app.press_key("Escape").unwrap();
    app.wait_for_gone(CONNECTIONS, WAIT).unwrap();
}

#[test]
fn the_connection_panel_previews_under_the_pointer_and_stays_when_pressed() {
    let mut app = start_shell();
    let rest = std::time::Duration::from_millis(600);

    // The pointer resting on the indicator opens a preview, which goes
    // once it has left.
    app.hover(INDICATOR).unwrap();
    app.wait_for(CONNECTIONS).unwrap();
    app.hover("id:clock").unwrap();
    app.wait_for_gone(CONNECTIONS, WAIT).unwrap();

    // Moving onto the preview keeps it.
    app.hover(INDICATOR).unwrap();
    app.wait_for(CONNECTIONS).unwrap();
    app.hover(CONNECTIONS).unwrap();
    app.wait(rest).unwrap();
    assert!(app.screen().matches(CONNECTIONS).unwrap());
    app.hover("id:clock").unwrap();
    app.wait_for_gone(CONNECTIONS, WAIT).unwrap();

    // A press keeps it wherever the pointer goes, until it is closed.
    app.hover(INDICATOR).unwrap();
    app.wait_for(CONNECTIONS).unwrap();
    app.press(INDICATOR).unwrap();
    app.hover("id:clock").unwrap();
    app.wait(rest).unwrap();
    assert!(app.screen().matches(CONNECTIONS).unwrap());
    app.press(INDICATOR).unwrap();
    app.wait_for_gone(CONNECTIONS, WAIT).unwrap();
}

const SEARCH: &str = "id:symbol-search";
const RESULTS: &str = "id:symbol-results";

#[test]
fn a_pick_in_the_search_charts_it_on_the_widgets_linked_to_the_bars_color() {
    let mut app = start_shell();

    // The field opens its results when it is typed in: every BTC/USDC, the
    // most traded first, perpetuals before the spot pairs nobody ranks.
    app.type_text(SEARCH, "btc/usdc").unwrap();
    app.wait_for(RESULTS).unwrap();
    app.wait_for("Hyperliquid BTC/USDC PERP USDC-M").unwrap();
    assert_results(
        &mut app,
        &[
            "Hyperliquid BTC/USDC PERP USDC-M",
            "Bybit BTC/USDC PERP USDC-M",
            "Bybit BTC/USDC SPOT",
            "Hyperliquid BTC/USDC SPOT",
        ],
    );
    assert_eq!(app.screen().count("role:option").unwrap(), 4);

    // The chart is linked to blue, as the bar is: the pick becomes its
    // market, the field empties and the results go.
    app.press("Hyperliquid BTC/USDC PERP USDC-M").unwrap();
    app.wait_for("BTC/USDC  ·  Hyperliquid").unwrap();
    app.wait_for_gone(RESULTS, WAIT).unwrap();
    assert_eq!(app.screen().input_text(SEARCH).unwrap(), Some(""));

    // The bar's link button lists the colors and how many widgets each has.
    app.press("id:bar-link").unwrap();
    app.wait_for("id:link-menu").unwrap();
    assert!(app.shows("1 widget"));
    app.press("role:option[Red]").unwrap();
    app.wait_for_gone("id:link-menu", WAIT).unwrap();

    // Nothing is linked to red: the results say so, and Enter on the first
    // of them changes no widget.
    app.type_text(SEARCH, "eth").unwrap();
    app.wait_for("Hyperliquid ETH/USDC PERP USDC-M").unwrap();
    assert!(app.shows(
        "No widget is linked to Red. Link one from its header's link button to send it markets."
    ));
    app.type_text(SEARCH, "\n").unwrap();
    app.wait_for_gone(RESULTS, WAIT).unwrap();
    assert!(app.shows("BTC/USDC  ·  Hyperliquid"));

    // The chart's header links it to red; the next pick is its market.
    app.press("id:pane-link-1").unwrap();
    app.wait_for("id:link-menu").unwrap();
    app.press("role:option[Red]").unwrap();
    app.wait_for_gone("id:link-menu", WAIT).unwrap();
    app.type_text(SEARCH, "eth").unwrap();
    app.wait_for("Hyperliquid ETH/USDC PERP USDC-M").unwrap();
    // Down moves the keyboard's row to the second result, Bybit's.
    app.press_key("down").unwrap();
    app.type_text(SEARCH, "\n").unwrap();
    app.wait_for("ETH/USDT  ·  Bybit").unwrap();

    // The chart's settings have the same choice, as the old panel did.
    app.press("Widget settings").unwrap();
    app.wait_for("id:chart-settings").unwrap();
    app.press("Link Color").unwrap();
    app.press("Green").unwrap();
    app.wait_for("label:Widget link color: Green").unwrap();
}

#[test]
fn a_market_picker_offers_the_markets_the_bar_keeps() {
    let mut app = start_shell();

    // The chart's market picker reads the directory the bar holds: its
    // exchanges' markets are there, and a pick charts as before.
    app.press("Widget settings").unwrap();
    app.wait_for("id:chart-settings").unwrap();
    app.press("id:chart-market").unwrap();
    app.wait_for("id:market-picker").unwrap();
    app.wait_for("Hyperliquid ETH/USDC PERP USDC-M").unwrap();
    assert_eq!(app.screen().count("role:option").unwrap(), 7);
    app.press("Hyperliquid ETH/USDC PERP USDC-M").unwrap();
    app.wait_for("ETH/USDC  ·  Hyperliquid").unwrap();
    app.wait_for_gone("id:market-picker", WAIT).unwrap();
}

#[test]
fn the_search_lists_the_most_traded_markets_until_something_is_typed() {
    let mut app = start_shell();

    // Focused with nothing typed: every exchange's most traded markets.
    app.type_text(SEARCH, "").unwrap();
    app.wait_for(RESULTS).unwrap();
    app.wait_for("Hyperliquid BTC/USDC PERP USDC-M").unwrap();
    assert_results(
        &mut app,
        &[
            "Hyperliquid BTC/USDC PERP USDC-M",
            "Hyperliquid ETH/USDC PERP USDC-M",
            "Bybit BTC/USDT PERP USDT-M",
            "Bybit ETH/USDT PERP USDT-M",
            "Bybit BTC/USDC PERP USDC-M",
        ],
    );
    assert_eq!(app.screen().count("role:option").unwrap(), 7);

    // An exchange and a pair, in words.
    app.type_text(SEARCH, "hyper eth").unwrap();
    app.wait_for_gone("Bybit BTC/USDT PERP USDT-M", WAIT)
        .unwrap();
    assert_results(&mut app, &["Hyperliquid ETH/USDC PERP USDC-M"]);
    assert_eq!(app.screen().count("role:option").unwrap(), 1);

    // Nothing matches: it says so. Escape empties the field and closes.
    app.clear(SEARCH).unwrap();
    app.type_text(SEARCH, "zzz").unwrap();
    app.wait_for("No symbols found for \"zzz\"").unwrap();
    app.press_key("Escape").unwrap();
    app.wait_for_gone(RESULTS, WAIT).unwrap();
    assert_eq!(app.screen().input_text(SEARCH).unwrap(), Some(""));
}

/// A Hyperliquid no test reaches: its HTTP is the test's stub network.
const HYPERLIQUID: &str = "https://hyperliquid.test";
/// The wallet's cell of the bar and its panel.
const WALLET: &str = "id:wallet";
const WALLET_PANEL: &str = "id:wallet-panel";
/// The account the stand-in wallets share, whole and in short.
const ACCOUNT: &str = "0x14791697260e4c9a71f18484c9f997b308e59325";
const ACCOUNT_SHORT: &str = "0x1479…9325";

/// One wallet in the page, as the web build finds it.
#[component]
fn OneWallet() -> impl IntoElement {
    wallet_bar(Reach::Page, &["Rabby Wallet"])
}

/// Two wallets in the user's browser, as the desktop build finds them.
#[component]
fn BrowserWallets() -> impl IntoElement {
    wallet_bar(Reach::Browser, &["Rabby Wallet", "MetaMask"])
}

/// A browser without a wallet.
#[component]
fn NoWallet() -> impl IntoElement {
    wallet_bar(Reach::Page, &[])
}

/// The bar over data services whose wallets are stand-ins and whose
/// Hyperliquid is the stub network, with buttons under it that make the
/// stand-ins act as a user or a wallet would.
fn wallet_bar(reach: Reach, names: &[&str]) -> View {
    let wallets = StandIn::new(reach, names);
    assert_eq!(format!("{:#x}", wallets.address()), ACCOUNT);
    DataServices::provide_with(DataConfig {
        ferris: FerrisConfig {
            http_base: FERRIS.to_owned(),
            ws_url: "ws://127.0.0.1:9/v1/ws".to_owned(),
            ..FerrisConfig::production()
        },
        hyperliquid: HyperliquidConfig {
            http_base: HYPERLIQUID.to_owned(),
            ..HyperliquidConfig::production()
        },
        wallet: WalletConfig {
            connector: Some(Rc::new(wallets.clone())),
        },
        ..DataConfig::default()
    });
    let mut saved = StoredWorkspace::default();
    layout::normalize(&mut saved);
    let workspace = signal(saved);
    let acts = |label: &'static str, act: Box<dyn Fn(&StandIn)>| {
        let wallets = wallets.clone();
        view()
            .role(AccessibilityRole::Button)
            .label(label)
            .on_press(move || act(&wallets))
            .child(text(label))
    };
    let says = |label: &'static str, event: Event| {
        acts(label, Box::new(move |wallets| wallets.say(event.clone())))
    };
    let first = "w1".to_owned();
    view()
        .size_full()
        .flex_col()
        .child(bar(
            workspace,
            ActiveTheme::restore(),
            Clock::restore(),
            crate::widgets::picker::Picker::new(),
        ))
        .child(acts(
            "the user rejects",
            Box::new(|wallets| wallets.reject(true)),
        ))
        .child(acts(
            "the user approves",
            Box::new(|wallets| wallets.reject(false)),
        ))
        .child(acts(
            "another key signs",
            Box::new(StandIn::sign_as_another),
        ))
        .child(acts(
            "what was asked",
            Box::new(|wallets| drop(set_clipboard(wallets.asked().join(" ")))),
        ))
        .child(says(
            "the wallet changes chain",
            Event::Chain {
                wallet: first.clone(),
                chain: json!("0x1"),
            },
        ))
        .child(says(
            "the wallet changes account",
            Event::Accounts {
                wallet: first.clone(),
                accounts: json!(["0x5e9ee1089755c3435139848e47e6635505d5a13a"]),
            },
        ))
        .child(says(
            "another wallet changes account",
            Event::Accounts {
                wallet: "w2".to_owned(),
                accounts: json!([]),
            },
        ))
        .child(says(
            "the wallet disconnects",
            Event::Accounts {
                wallet: first,
                accounts: json!([]),
            },
        ))
        .child(says(
            "the page closes",
            Event::Lost("The wallet page was closed.".to_owned()),
        ))
}

/// A request's JSON body.
fn body_of(request: &HttpRequest) -> Value {
    match &request.body {
        HttpBody::Bytes(bytes) => serde_json::from_slice(bytes).unwrap_or_default(),
        _ => Value::Null,
    }
}

/// Answers the stubbed Hyperliquid's exchange endpoint: whatever is
/// posted was done, and an agent approved is the one `listed` from then on
/// (an account has one of a name).
fn stub_hyperliquid_exchange(
    listed: &Mutex<Option<String>>,
    request: &HttpRequest,
    reply: StubReply,
) {
    let action = &body_of(request)["action"];
    if action["type"] == "approveAgent" {
        *listed.lock().unwrap() = action["agentAddress"].as_str().map(str::to_owned);
    }
    let mut response = StubResponse::default();
    response.status = 200;
    response.body = br#"{"status":"ok","response":{"type":"default"}}"#.to_vec();
    reply.send(response);
}

/// Answers the stubbed Hyperliquid's info requests: the agent `listed`,
/// valid for years, and nothing else.
fn stub_hyperliquid_info(listed: &Mutex<Option<String>>, request: &HttpRequest, reply: StubReply) {
    let mut response = StubResponse::default();
    if body_of(request)["type"] == "extraAgents" {
        let agents: Value = listed
            .lock()
            .unwrap()
            .iter()
            .map(|agent| {
                json!({ "name": "Lattice", "address": agent, "validUntil": 4_102_444_800_000_u64 })
            })
            .collect();
        response.status = 200;
        response.body = serde_json::to_vec(&agents).expect("the agents are JSON");
    } else {
        response.status = 422;
        response.body = b"Failed to deserialize the JSON body into the target type".to_vec();
    }
    reply.send(response);
}

fn start_wallet(root: Root, os: Os) -> Headless {
    start_wallet_in(root, os, 1280.0)
}

/// [`start_wallet`] in a window `width` wide.
fn start_wallet_in(root: Root, os: Os, width: f32) -> Headless {
    start_trading_in(root, os, width).0
}

/// [`start_wallet`] with the stub network, which keeps what was posted to
/// Hyperliquid.
fn start_trading(root: Root, os: Os) -> (Headless, Arc<StubNetwork>) {
    start_trading_in(root, os, 1280.0)
}

fn start_trading_in(root: Root, os: Os, width: f32) -> (Headless, Arc<StubNetwork>) {
    let listed: Arc<Mutex<Option<String>>> = Arc::default();
    let (approves, lists) = (listed.clone(), listed);
    let network = Arc::new(StubNetwork::new([
        (
            format!("POST {HYPERLIQUID}/exchange"),
            StubRoute::Handler(Arc::new(move |request: HttpRequest, reply: StubReply| {
                stub_hyperliquid_exchange(&approves, &request, reply);
            })),
        ),
        (
            format!("POST {HYPERLIQUID}/info"),
            StubRoute::Handler(Arc::new(move |request: HttpRequest, reply: StubReply| {
                stub_hyperliquid_info(&lists, &request, reply);
            })),
        ),
    ]));
    let mut config = EngineConfig::new(root);
    config.viewport = Size::new(px(width), px(800.0));
    config.storage = Some(MemoryStorage::new([]));
    config.network = Some(network.clone());
    config.environment.os = os;
    config.environment.idiom = Idiom::Desktop;
    let mut app = Headless::start(config).unwrap();
    app.wait_for(WALLET).unwrap();
    (app, network)
}

/// What was posted to Hyperliquid's exchange endpoint so far, in order.
fn posted(network: &StubNetwork) -> Vec<Value> {
    let exchange = format!("{HYPERLIQUID}/exchange");
    let requests = network.requests();
    let posted = requests.iter().filter(|request| request.url == exchange);
    posted.map(body_of).collect()
}

/// How many times Hyperliquid was asked which agents the account has.
fn agents_asked(network: &StubNetwork) -> usize {
    let info = format!("{HYPERLIQUID}/info");
    let requests = network.requests();
    let asked = requests.iter().filter(|request| request.url == info);
    asked
        .filter(|request| body_of(request)["type"] == "extraAgents")
        .count()
}

/// The stand-in wallet's Hyperliquid row, and what it says while trading
/// is on or off.
const ENABLE: &str = "label:Enable trading on Hyperliquid";
const DISABLE: &str = "label:Disable trading on Hyperliquid";

/// Whether the bar's cell says no wallet is connected.
fn disconnected(app: &Headless) -> bool {
    app.screen().matches("label:Wallet: not connected").unwrap()
}

/// Opens the wallet's panel.
fn open_wallet(app: &mut Headless) {
    app.press(WALLET).unwrap();
    app.wait_for(WALLET_PANEL).unwrap();
}

/// Connects the one wallet there is, from the bar.
fn connect_wallet(app: &mut Headless) {
    open_wallet(app);
    app.press("Connect wallet").unwrap();
    app.wait_for("Rabby Wallet").unwrap();
    app.wait_for(ACCOUNT_SHORT).unwrap();
}

/// Connects the one wallet there is again, with the panel open.
fn connect_again(app: &mut Headless) {
    app.press("Connect wallet").unwrap();
    app.wait_for(ACCOUNT_SHORT).unwrap();
}

/// What the stand-in wallets were asked, in order.
fn asked(app: &mut Headless) -> String {
    app.press("what was asked").unwrap();
    app.screen().clipboard().unwrap_or_default().to_owned()
}

/// Where layout put the view whose test id is `id`.
fn placed(app: &Headless, id: &str) -> Bounds<Points> {
    let nodes = app.screen().layout_nodes().into_iter();
    nodes
        .filter(|node| node.test_id.as_deref() == Some(id))
        .map(|node| node.bounds)
        .next()
        .unwrap_or_else(|| panic!("{id} is not on screen"))
}

#[test]
fn the_wallet_stands_between_the_wordmark_and_the_layout_button() {
    for os in [Os::Windows, Os::Web] {
        let app = start_wallet(nsh_testing::windowed_root::<OneWallet>(), os);
        let [wordmark, wallet, layout] =
            ["wordmark", "wallet", "layout-button"].map(|id| placed(&app, id));
        let right = |cell: Bounds<Points>| cell.origin.x.0 + cell.size.width.0;
        // Each cell begins where the one before it ends, and is as high
        // as the bar inside its border.
        assert!((right(wordmark) - wallet.origin.x.0).abs() < 0.5, "{os:?}");
        assert!((right(wallet) - layout.origin.x.0).abs() < 0.5, "{os:?}");
        assert_eq!(wallet.size.height, layout.size.height, "{os:?}");
        assert_eq!(wallet.size.height, wordmark.size.height, "{os:?}");
        assert!(disconnected(&app));
        // With two pages the group leaves the middle of the bar (the link
        // button, the search and New Widget) in the middle of the bar.
        let [bar, link, new_widget] =
            ["everything-bar", "bar-link", "new-widget"].map(|id| placed(&app, id));
        let middle = (link.origin.x.0 + right(new_widget)) / 2.0;
        let bar_middle = bar.origin.x.0 + bar.size.width.0 / 2.0;
        assert!((middle - bar_middle).abs() < 1.0, "{os:?}");
        assert!(app.shows("Connect"));
    }
    // A wide window has room to say it in full.
    let app = start_wallet_in(nsh_testing::windowed_root::<OneWallet>(), Os::Web, 1440.0);
    assert!(app.shows("Connect Wallet"));
}

#[test]
fn the_one_wallet_there_is_connects_and_enables_trading() {
    let (mut app, network) = start_trading(nsh_testing::windowed_root::<OneWallet>(), Os::Web);
    connect_wallet(&mut app);
    // The bar's cell is the account now, and the panel says what is
    // connected.
    assert!(!disconnected(&app));
    assert!(
        app.screen()
            .matches(&format!("label:Wallet: {ACCOUNT}"))
            .unwrap()
    );
    assert!(app.shows("Arbitrum One (42161)"));
    assert_eq!(asked(&mut app), "eth_requestAccounts eth_chainId");
    app.press("Copy the account's address").unwrap();
    assert_eq!(app.screen().clipboard(), Some(ACCOUNT));

    // Each exchange it can trade on is listed with where it stands there:
    // nothing trades for it yet, and what enabling asks is said before it
    // is asked.
    assert!(app.shows("Hyperliquid") && app.shows("Trading off"));
    assert!(app.screen().texts().iter().any(|text| {
        text.starts_with("Your wallet signs one approval.")
            && text.ends_with("The key cannot withdraw or transfer.")
    }));
    assert!(posted(&network).is_empty());

    // Enabling: the wallet signs one approval, which is posted.
    app.press(ENABLE).unwrap();
    app.wait_for("Trading on").unwrap();
    assert_eq!(
        asked(&mut app),
        "eth_requestAccounts eth_chainId eth_signTypedData_v4"
    );
    let requests = posted(&network);
    let [approval] = &requests[..] else {
        panic!("one approval is posted, not {requests:?}");
    };
    let action = &approval["action"];
    assert_eq!(action["type"], "approveAgent");
    assert_eq!(action["hyperliquidChain"], "Mainnet");
    // The chain the wallet signed on, Arbitrum One.
    assert_eq!(action["signatureChainId"], "0xa4b1");
    assert_eq!(action["agentName"], "Lattice");
    assert_eq!(action["nonce"], approval["nonce"]);
    assert_eq!(approval["vaultAddress"], Value::Null);
    // What was posted is what the account signed: read as Hyperliquid
    // reads it, the signer is the account, and the agent is a key of the
    // app's own.
    let agent = action["agentAddress"].as_str().unwrap();
    assert_ne!(agent, ACCOUNT);
    let signed = signing::approve_agent(
        signing::Network::Mainnet,
        42_161,
        agent.parse().unwrap(),
        "Lattice",
        approval["nonce"].as_u64().unwrap(),
    )
    .unwrap();
    let signature = &approval["signature"];
    let word = |name: &str| signature[name].as_str().unwrap().parse().unwrap();
    let signature = alloy::primitives::Signature::new(
        word("r"),
        word("s"),
        signature["v"].as_u64().unwrap() == 28,
    );
    assert_eq!(
        signing::signer_of(&signed, &signature).map(|signer| format!("{signer:#x}")),
        Ok(ACCOUNT.to_owned())
    );

    // Disabling forgets the agent for good; enabling again approves a new
    // one, which the wallet signs for.
    app.press(DISABLE).unwrap();
    app.wait_for("Trading off").unwrap();
    app.press(ENABLE).unwrap();
    app.wait_for("Trading on").unwrap();
    let again = posted(&network);
    assert_eq!(again.len(), 2);
    assert_ne!(again[1]["action"]["agentAddress"], agent);
    assert_eq!(agents_asked(&network), 0, "nothing was kept to ask about");

    // Closing the panel changes nothing of the connection; Disconnect ends
    // it, and nothing trades for an account that is not connected.
    app.press("Close Wallet").unwrap();
    app.wait_for_gone(WALLET_PANEL, WAIT).unwrap();
    assert!(app.shows(ACCOUNT_SHORT));
    open_wallet(&mut app);
    assert!(app.shows("Trading on"));
    app.press("Disconnect").unwrap();
    app.wait_for("label:Wallet: not connected").unwrap();
    assert!(app.shows("Connect wallet"));
    assert!(!app.shows("Trading on"));

    // Connected again, what the account allowed is taken up: Hyperliquid
    // is asked whether the agent still holds, and the wallet signs nothing
    // more.
    connect_again(&mut app);
    app.wait_for("Trading on").unwrap();
    assert_eq!(agents_asked(&network), 1);
    assert_eq!(posted(&network).len(), 2);
    assert_eq!(
        asked(&mut app),
        "eth_requestAccounts eth_chainId eth_signTypedData_v4 eth_signTypedData_v4 \
         eth_requestAccounts eth_chainId"
    );

    // Disabled, it is not taken up again: nothing is kept to ask about.
    app.press(DISABLE).unwrap();
    app.wait_for("Trading off").unwrap();
    app.press("Disconnect").unwrap();
    app.wait_for("label:Wallet: not connected").unwrap();
    connect_again(&mut app);
    assert!(app.shows("Trading off"));
    assert_eq!(agents_asked(&network), 1);
    assert_eq!(posted(&network).len(), 2);
}

#[test]
fn what_the_user_turns_down_is_said_and_can_be_tried_again() {
    let (mut app, network) = start_trading(nsh_testing::windowed_root::<OneWallet>(), Os::Web);
    app.press("the user rejects").unwrap();
    open_wallet(&mut app);
    app.press("Connect wallet").unwrap();
    // Not connected: the wallet is offered again, with why.
    app.wait_for("Rejected in the wallet.").unwrap();
    assert!(app.shows("Connecting…"));
    app.press("the user approves").unwrap();
    app.press("Connect Rabby Wallet").unwrap();
    app.wait_for(ACCOUNT_SHORT).unwrap();

    // An approval turned down enables nothing, and nothing is posted.
    app.press("the user rejects").unwrap();
    app.press(ENABLE).unwrap();
    app.wait_for("Hyperliquid: Rejected in the wallet.")
        .unwrap();
    assert!(app.shows("Trading off"));
    assert!(app.shows(ACCOUNT_SHORT));
    assert!(posted(&network).is_empty());

    // And one signed by another key is not this account's: Hyperliquid
    // would take it for that key's account, so it is never sent.
    app.press("the user approves").unwrap();
    app.press("another key signs").unwrap();
    app.press(ENABLE).unwrap();
    app.wait_until("the other signer being named", WAIT, |screen| {
        screen.texts().iter().any(|text| {
            text.starts_with("Hyperliquid: Signed by 0x") && text.contains("not by this account")
        })
    })
    .unwrap();
    assert!(app.shows("Trading off"));
    assert!(posted(&network).is_empty());
}

#[test]
fn the_wallets_of_the_browser_are_chosen_from_on_desktop() {
    let mut app = start_wallet(nsh_testing::windowed_root::<BrowserWallets>(), Os::Windows);
    open_wallet(&mut app);
    assert!(app.shows(
        "A page opens in your browser, where your wallet is. Keep it open while connected."
    ));
    app.press("Connect wallet in browser").unwrap();
    // The page was opened in the browser, and both wallets it found are
    // offered.
    app.wait_for("Connect MetaMask").unwrap();
    assert_eq!(
        app.screen().opened_urls(),
        ["http://127.0.0.1:47821/secret"]
    );
    assert!(app.shows("Connecting…"));
    assert_eq!(asked(&mut app), "");

    app.press("Connect MetaMask").unwrap();
    app.wait_for(ACCOUNT_SHORT).unwrap();
    assert!(app.shows("MetaMask"));
    // What another wallet says is not this connection's.
    app.press("the wallet changes chain").unwrap();
    app.press("the wallet changes account").unwrap();
    app.press("the wallet disconnects").unwrap();
    assert!(app.shows(ACCOUNT_SHORT));
    assert!(app.shows("Arbitrum One (42161)"));

    // The page closing ends it, and says so.
    app.press("the page closes").unwrap();
    app.wait_for("The wallet page was closed.").unwrap();
    assert!(disconnected(&app));
}

#[test]
fn the_wallet_keeps_the_connection_current() {
    let mut app = start_wallet(nsh_testing::windowed_root::<OneWallet>(), Os::Web);
    connect_wallet(&mut app);
    app.press(ENABLE).unwrap();
    app.wait_for("Trading on").unwrap();

    // The chain the wallet is on is not who it is: trading stays on.
    app.press("the wallet changes chain").unwrap();
    app.wait_for("Ethereum (1)").unwrap();
    assert!(app.shows("Trading on"));
    // Another wallet's news changes nothing.
    app.press("another wallet changes account").unwrap();
    assert!(app.shows(ACCOUNT_SHORT));

    // Another account has allowed nothing: what the last one's agent was
    // approved for is not its to use.
    app.press("the wallet changes account").unwrap();
    app.wait_for("0x5e9e…a13a").unwrap();
    assert!(app.shows("Trading off") && !app.shows("Trading on"));

    // The site disconnected in the wallet is the end of it.
    app.press("the wallet disconnects").unwrap();
    app.wait_for("Rabby Wallet disconnected Lattice.").unwrap();
    assert!(disconnected(&app));
}

#[test]
fn a_browser_without_a_wallet_says_so() {
    let mut app = start_wallet(nsh_testing::windowed_root::<NoWallet>(), Os::Web);
    open_wallet(&mut app);
    app.press("Connect wallet").unwrap();
    app.wait_until("the missing wallet being said", WAIT, |screen| {
        screen
            .texts()
            .iter()
            .any(|text| text.starts_with("No wallet was found in this browser."))
    })
    .unwrap();
    app.press("Cancel").unwrap();
    app.wait_for("Connect wallet").unwrap();
    assert!(disconnected(&app));
}
