use nsh_core::{element::Root, prelude::*};
use nsh_testing::{Headless, WAIT};
use serde_json::json;

use super::{
    session::{self, Choice, EMPTY, LOADING, NONE, Readout, Sign, Standing, UNAVAILABLE, UNLISTED},
    settings::{DEFAULT_PROTOCOL, ProtocolTvlSettings, Range, Style},
};
use crate::{
    data::{
        ferris::defillama::{ProtocolTvl, Protocols, TvlPoint},
        net::ApiError,
    },
    format,
    widgets::{
        WidgetKind,
        common::{
            defillama::{DAYS, LATEST_TVL, asked, day_tvl, start, workspace_of},
            plot::Mode,
        },
    },
};

#[test]
fn settings_save_only_what_differs_and_read_leniently() {
    // Defaults save nothing: Hyperliquid, a year, a line, with the grid.
    let defaults = ProtocolTvlSettings::default();
    assert_eq!(serde_json::to_value(&defaults).unwrap(), json!({}));
    assert_eq!(defaults.protocol(), DEFAULT_PROTOCOL);
    assert_eq!(
        (defaults.range(), defaults.style()),
        (Range::Year, Style::Line)
    );
    assert!(defaults.grid());
    let written = json!({
        "protocol": "hyperliquid", "range": "1y", "style": "line", "showGrid": true,
    });
    assert_eq!(ProtocolTvlSettings::from(written), defaults);

    // Each choice under its id; a slug is kept as requests name it.
    let saved = json!({
        "protocol": "aave-v3", "range": "all", "style": "area", "showGrid": false,
    });
    let settings = ProtocolTvlSettings::from(saved.clone());
    assert_eq!(settings.protocol(), "aave-v3");
    assert_eq!(
        (settings.range(), settings.style()),
        (Range::All, Style::Area)
    );
    assert!(!settings.grid());
    assert_eq!(serde_json::to_value(&settings).unwrap(), saved);
    assert_eq!(
        settings.clone().with_protocol(" Trade[XYZ] ").protocol(),
        "trade[xyz]"
    );
    // Back at its default a choice is not saved.
    assert_eq!(
        serde_json::to_value(
            settings
                .with_protocol("Hyperliquid")
                .with_range(Range::Year)
                .with_style(Style::Line)
                .with_grid(true)
        )
        .unwrap(),
        json!({})
    );

    // What it does not know, or of the wrong type, is the default.
    let damaged = ProtocolTvlSettings::from(json!({
        "protocol": 7, "range": "5y", "style": null, "showGrid": "no",
    }));
    assert_eq!(damaged, defaults);
    assert_eq!(ProtocolTvlSettings::from(json!("aave")), defaults);
    assert_eq!(defaults.clone().with_protocol("  "), defaults);
    assert!(
        Range::ALL
            .iter()
            .all(|range| Range::from_id(range.id()) == Some(*range))
    );
    assert!(
        Style::ALL
            .iter()
            .all(|style| Style::from_id(style.id()) == Some(*style))
    );
}

const DAY: f64 = 86_400_000.0;

/// A protocol whose history is `points` (days since some day, dollars).
fn tvl_of(category: Option<&str>, points: &[(f64, f64)]) -> ProtocolTvl {
    let mut tvl = ProtocolTvl::default();
    tvl.protocol.category = category.map(str::to_owned);
    tvl.history = points
        .iter()
        .map(|(day, usd)| TvlPoint {
            time_ms: day * DAY,
            usd: *usd,
        })
        .collect();
    tvl
}

#[test]
fn a_range_plots_back_from_the_latest_point_and_reads_how_far_it_moved() {
    // 400 days, then a reading later the last day.
    let mut points: Vec<(f64, f64)> = (0..400)
        .map(|day| (f64::from(day), 100.0 + f64::from(day)))
        .collect();
    points.push((399.5, 600.0));
    let tvl = tvl_of(None, &points);

    // All of it: a bar a point, whole seconds, one value each.
    let all = session::bars_of(&tvl.history, Range::All);
    assert_eq!(all.len(), 401);
    assert_eq!((all[0].time, all[0].close), (0.0, 100.0));
    assert!(
        all.iter()
            .all(|bar| bar.open == bar.close && bar.high == bar.low)
    );
    // A range reaches back from the latest point, not from now.
    let month = session::bars_of(&tvl.history, Range::Month);
    assert_eq!(month.len(), 31);
    assert_eq!(month[0].close, 100.0 + 370.0);
    assert_eq!(session::bars_of(&tvl.history, Range::Quarter).len(), 91);
    assert_eq!(session::bars_of(&tvl.history, Range::Year).len(), 366);
    assert!(session::bars_of(&[], Range::Year).is_empty());

    // The latest figure, and the change over what is plotted.
    let readout = Readout::of(Some(&tvl), &month);
    assert_eq!(readout.measure, "TVL");
    assert_eq!(readout.value, "$600");
    assert_eq!(readout.change, format::signed_percent(600.0 / 470.0 - 1.0));
    assert_eq!(readout.sign, Sign::Up);
    let fell = tvl_of(Some("CEX"), &[(0.0, 2.0e9), (1.0, 1.5e9)]);
    let readout = Readout::of(Some(&fell), &session::bars_of(&fell.history, Range::All));
    assert_eq!(
        (
            readout.measure,
            readout.value.as_str(),
            readout.change.as_str(),
            readout.sign
        ),
        ("Assets", "$1.500B", "-25.00%", Sign::Down)
    );
    // One point, a range that began at nothing, or no point: no change.
    let lone = tvl_of(None, &[(0.0, 5.0)]);
    let readout = Readout::of(Some(&lone), &session::bars_of(&lone.history, Range::All));
    assert_eq!(
        (readout.value.as_str(), readout.change.as_str()),
        ("$5", NONE)
    );
    let born = tvl_of(None, &[(0.0, 0.0), (1.0, 5.0)]);
    let readout = Readout::of(Some(&born), &session::bars_of(&born.history, Range::All));
    assert_eq!((readout.change.as_str(), readout.sign), (NONE, Sign::Flat));
    let nothing = Readout::of(None, &[]);
    assert_eq!(
        (nothing.value.as_str(), nothing.change.as_str()),
        (NONE, NONE)
    );

    assert_eq!(session::mode_of(Style::Line), Mode::Signed);
    assert_eq!(session::mode_of(Style::Area), Mode::SignedArea);
}

#[test]
fn the_widget_says_how_its_data_stands_and_where_its_value_is() {
    let tvl = ProtocolTvl::read(&json!({
        "protocol": { "slug": "aave-v3", "name": "Aave V3" },
        "chainTvls": { "Ethereum": 80.0, "Arbitrum": 10.0, "Base": 6.0, "Polygon": 3.0, "Sonic": 1.0 },
        "extraTvls": { "borrowed": 2_500_000.0 },
        "tvlHistory": [[1_000, 100.0]],
    }))
    .unwrap();
    assert_eq!(
        session::chains_line(&tvl),
        "Ethereum 80.0% · Arbitrum 10.0% · Base 6.0% · +2 more"
    );
    let detail = session::chains_detail(&tvl);
    assert_eq!(detail.lines().count(), 7);
    assert!(detail.contains("Sonic $1") && detail.ends_with("borrowed $2.500M"));
    let bare = ProtocolTvl::default();
    assert_eq!(session::chains_line(&bare), "");
    assert_eq!(session::chains_detail(&bare), "");

    // How a request stands, and what is said of it.
    let failure = ApiError::network("offline");
    let unlisted = ApiError::api(Some(404), "PROTOCOL_NOT_FOUND", "not listed");
    let stale = ProtocolTvl {
        stale: true,
        ..tvl.clone()
    };
    assert_eq!(Standing::of(None, None), Standing::Loading);
    assert_eq!(Standing::of(Some(&tvl), None), Standing::Ready);
    assert_eq!(Standing::of(Some(&stale), None), Standing::Stale);
    assert_eq!(
        Standing::of(Some(&tvl), Some(&failure)),
        Standing::Behind("offline".to_owned())
    );
    assert_eq!(
        Standing::of(None, Some(&failure)),
        Standing::Failed("offline".to_owned())
    );
    assert_eq!(Standing::of(None, Some(&unlisted)), Standing::Unlisted);
    assert_eq!(Standing::Loading.said(false), LOADING);
    assert_eq!(Standing::Failed(String::new()).said(false), UNAVAILABLE);
    assert_eq!(Standing::Unlisted.said(false), UNLISTED);
    assert_eq!(Standing::Ready.said(false), EMPTY);
    assert_eq!(Standing::Ready.said(true), "");
    // Only what is wrong warns; a slug DefiLlama does not list is said in
    // the plot's middle.
    assert!(Standing::Ready.warning().is_none() && Standing::Unlisted.warning().is_none());
    assert!(Standing::Stale.warning().is_some());
    assert!(
        Standing::Behind("offline".to_owned())
            .warning()
            .is_some_and(|warning| warning.contains("offline"))
    );

    assert_eq!(session::titled("apex-protocol"), "Apex Protocol");
    assert_eq!(session::titled("hyperliquid"), "Hyperliquid");
}

#[test]
fn the_picker_offers_the_exchanges_and_a_parent_before_its_products() {
    // Before anything is typed: the exchanges Lattice shows, by their slug.
    let venues = session::venue_choices();
    let binance = venues
        .iter()
        .find(|choice| choice.exchange == Some("binance"))
        .expect("Binance is listed");
    assert_eq!(
        (binance.slug.as_str(), binance.name.as_str()),
        ("binance-cex", "Binance")
    );
    assert!(venues.iter().any(|choice| choice.slug == "hyperliquid"));

    let found = Protocols::read(&json!({ "protocols": [
        { "slug": "hyperliquid-bridge", "name": "Hyperliquid Bridge", "category": "Bridge",
          "tvl": 5.6e9, "parentSlug": "hyperliquid" },
        { "slug": "hyperliquid-earn", "name": "Hyperliquid Earn", "category": "Lending",
          "tvl": 1.4e9, "parentSlug": "hyperliquid" },
        { "slug": "perpl", "name": "Perpl", "category": "Derivatives", "tvl": 2.9e6 },
    ] }))
    .unwrap();
    let choices = session::choices_of(&found);
    let read: Vec<(&str, &str, &str)> = choices
        .iter()
        .map(|choice: &Choice| {
            (
                choice.slug.as_str(),
                choice.name.as_str(),
                choice.note.as_str(),
            )
        })
        .collect();
    assert_eq!(
        read,
        [
            ("hyperliquid", "Hyperliquid", "All products"),
            ("hyperliquid-bridge", "Hyperliquid Bridge", "Bridge"),
            ("hyperliquid-earn", "Hyperliquid Earn", "Lending"),
            ("perpl", "Perpl", "Derivatives"),
        ]
    );
    assert_eq!((choices[0].tvl, choices[1].tvl), (None, Some(5.6e9)));
}

/// One charting its default protocol.
#[component]
fn Charting() -> impl IntoElement {
    workspace_of(WidgetKind::ProtocolTvl, None)
}

/// One charting a protocol DefiLlama lists for its volume only.
#[component]
fn Unlocked() -> impl IntoElement {
    workspace_of(
        WidgetKind::ProtocolTvl,
        Some(json!({ "protocol": "variational" })),
    )
}

/// One charting a slug DefiLlama does not list.
#[component]
fn Unknown() -> impl IntoElement {
    workspace_of(
        WidgetKind::ProtocolTvl,
        Some(json!({ "protocol": "nowhere" })),
    )
}

/// Whether the part `id` of the widget reads `read`.
fn reads(app: &Headless, id: &str, read: &str) -> bool {
    app.screen()
        .matches(&format!("id:protocol-tvl-{id} >> text:{read}"))
        .unwrap()
}

/// How far the stubbed Hyperliquid moved since the start of the day `ago`
/// days before its latest, as the widget writes it.
fn moved_since(ago: u64) -> String {
    format::signed_percent(LATEST_TVL / day_tvl(ago) - 1.0)
}

#[test]
fn the_plot_shows_the_range_its_settings_pick_of_one_reply() {
    let (mut app, network) = start(Root::of::<Charting>());

    // A year of Hyperliquid's TVL by default: its latest figure, how far it
    // moved since the range began, and where it is.
    app.wait_for("id:protocol-tvl-value >> text:$7.151B")
        .unwrap();
    assert!(reads(&app, "value", "TVL"));
    assert!(reads(&app, "protocol", "Hyperliquid"));
    assert!(reads(&app, "range", "1Y"));
    assert!(
        reads(&app, "change", &moved_since(364)),
        "{}",
        moved_since(364)
    );
    assert!(reads(
        &app,
        "chains",
        "Hyperliquid L1 98.9% · Arbitrum 1.1%"
    ));
    assert!(
        app.screen()
            .matches("id:protocol-tvl-credit >> text:DefiLlama")
            .unwrap()
    );
    assert!(app.screen().matches("label:Open DefiLlama").unwrap());
    assert!(!app.shows(LOADING) && !app.shows(EMPTY));
    assert!(!app.shows("Could not load") && !app.shows("DefiLlama is not answering"));

    // Another range is cut from the same reply.
    app.press("id:protocol-tvl-range").unwrap();
    app.wait_for("id:protocol-tvl-range-menu").unwrap();
    app.press("id:protocol-tvl-range-30d").unwrap();
    app.wait_for_gone("id:protocol-tvl-range-menu", WAIT)
        .unwrap();
    app.wait_for(&format!(
        "id:protocol-tvl-change >> text:{}",
        moved_since(29)
    ))
    .unwrap();
    assert!(reads(&app, "range", "30D"));
    // And from the settings panel: all of it.
    app.press("Widget settings").unwrap();
    app.wait_for("id:protocol-tvl-settings").unwrap();
    app.press("id:protocol-tvl-settings >> label:Range")
        .unwrap();
    app.press("role:option[All]").unwrap();
    app.wait_for(&format!(
        "id:protocol-tvl-change >> text:{}",
        moved_since(DAYS - 1)
    ))
    .unwrap();
    app.press("label:Close Protocol TVL settings").unwrap();
    app.wait_for_gone("id:protocol-tvl-settings", WAIT).unwrap();

    // One request carried every range.
    assert_eq!(asked(&network), ["/v1/defillama/protocol/hyperliquid"]);
}

#[test]
fn the_protocol_badge_opens_the_picker_which_lists_exchanges_and_searches() {
    let (mut app, network) = start(Root::of::<Charting>());
    app.wait_for("id:protocol-tvl-value >> text:$7.151B")
        .unwrap();

    // The picker lists the exchanges until something is typed, the one
    // charted marked.
    app.press("id:protocol-tvl-protocol").unwrap();
    app.wait_for("id:protocol-tvl-picker").unwrap();
    const FIELD: &str = "id:protocol-tvl-picker >> label:Search protocols";
    assert!(app.screen().matches("label:Chart Hyperliquid").unwrap());
    assert!(
        app.screen()
            .matches("id:protocol-tvl-pick-binance-cex")
            .unwrap()
    );

    // A search asks the list once typing has stopped, and puts a parent
    // before its products.
    app.type_text(FIELD, "hyperliquid").unwrap();
    app.wait_for("label:Chart Hyperliquid Bridge").unwrap();
    assert!(
        app.screen()
            .matches("id:protocol-tvl-pick-hyperliquid >> text:All products")
            .unwrap()
    );
    assert!(
        app.screen()
            .matches("id:protocol-tvl-pick-hyperliquid-perps")
            .unwrap()
    );
    assert!(
        !app.screen()
            .matches("id:protocol-tvl-pick-binance-cex")
            .unwrap()
    );
    let searches: Vec<String> = asked(&network)
        .into_iter()
        .filter(|request| request.contains("search="))
        .collect();
    assert_eq!(
        searches,
        ["/v1/defillama/protocols?search=hyperliquid&limit=20"]
    );
    // Nothing found says so.
    app.clear(FIELD).unwrap();
    app.type_text(FIELD, "zzz").unwrap();
    app.wait_for("No protocol found.").unwrap();

    // Emptied, it lists the exchanges again; a pick charts it and closes
    // the picker. A centralized exchange's figure is its assets.
    app.clear(FIELD).unwrap();
    app.wait_for("id:protocol-tvl-pick-binance-cex").unwrap();
    app.press("id:protocol-tvl-pick-binance-cex").unwrap();
    app.wait_for_gone("id:protocol-tvl-picker", WAIT).unwrap();
    app.wait_for("id:protocol-tvl-value >> text:$109.5B")
        .unwrap();
    assert!(reads(&app, "value", "Assets"));
    assert!(reads(&app, "protocol", "Binance CEX"));
    assert!(reads(&app, "chains", "Ethereum 54.8% · Bitcoin 45.2%"));
    assert!(
        asked(&network).contains(&"/v1/defillama/protocol/binance-cex".to_owned()),
        "{:?}",
        asked(&network)
    );
}

#[test]
fn a_protocol_with_no_tvl_or_that_is_not_listed_says_so() {
    let (mut app, _network) = start(Root::of::<Unlocked>());
    app.wait_for(EMPTY).unwrap();
    assert!(reads(&app, "protocol", "Variational"));
    assert!(reads(&app, "value", NONE));
    assert!(reads(&app, "chains", "No chain breakdown"));

    let (mut app, _network) = start(Root::of::<Unknown>());
    app.wait_for(UNLISTED).unwrap();
    // Until DefiLlama names it, it goes by its slug's words.
    assert!(reads(&app, "protocol", "Nowhere"));
}
