use std::sync::Arc;

use super::*;
use nsh_core::{
    element::Root,
    platform::{Idiom, Os},
};
use nsh_engine::{EngineConfig, HttpBody, HttpRequest};
use nsh_testing::{Headless, MemoryStorage, StubNetwork, StubReply, StubResponse, StubRoute, WAIT};
use serde_json::{Value, json};

use crate::{
    data::{DataServices, ferris::FerrisConfig},
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
        ..EngineConfig::new(Root::of::<Harness>())
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
    let mut config = EngineConfig::new(Root::of::<Harness>());
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
    start_root(Root::of::<Shell>())
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
    app.wait_for("BTC/USDT  ·  1m  ·  Bybit").unwrap();
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
    let mut app = start_root(Root::of::<TwoCharts>());

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
    app.wait_for("BTC/USDC  ·  1m  ·  Hyperliquid").unwrap();
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
    assert!(app.shows("BTC/USDC  ·  1m  ·  Hyperliquid"));

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
    app.wait_for("ETH/USDT  ·  1m  ·  Bybit").unwrap();

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
    app.wait_for("ETH/USDC  ·  1m  ·  Hyperliquid").unwrap();
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
