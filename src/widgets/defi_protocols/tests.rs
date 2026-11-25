use nsh_core::prelude::*;
use nsh_testing::{Headless, WAIT};
use serde_json::json;

use super::{
    session::{self, NONE, Standing, Table, Tone},
    settings::{Column, DefiProtocolsSettings},
    view,
};
use crate::{
    data::{ferris::defillama::Protocols, net::ApiError},
    shell::clock::ClockPrefs,
    widgets::{
        WidgetKind,
        common::defillama::{LATEST_MS, asked, start, workspace_of},
    },
};

#[test]
fn settings_save_only_what_differs_and_read_leniently() {
    // Defaults save nothing: every category, largest TVL first.
    let defaults = DefiProtocolsSettings::default();
    assert_eq!(serde_json::to_value(&defaults).unwrap(), json!({}));
    assert_eq!(defaults.category(), None);
    assert_eq!(
        (defaults.sort(), defaults.descending()),
        (Column::Tvl, true)
    );
    assert_eq!(
        DefiProtocolsSettings::from(json!({ "sort": "tvl", "reversed": false })),
        defaults
    );

    let saved = json!({ "category": "Liquid Staking", "sort": "name", "reversed": true });
    let settings = DefiProtocolsSettings::from(saved.clone());
    assert_eq!(settings.category(), Some("Liquid Staking"));
    // A name reads from A until reversed.
    assert_eq!(
        (settings.sort(), settings.descending()),
        (Column::Name, true)
    );
    assert_eq!(serde_json::to_value(&settings).unwrap(), saved);
    assert_eq!(
        serde_json::to_value(settings.with_category(None).sorted_by(Column::Tvl)).unwrap(),
        json!({})
    );

    // A title's press orders by its column, and again the other way round;
    // the rank orders nothing.
    let by_day = defaults.clone().sorted_by(Column::Change1d);
    assert_eq!(
        (by_day.sort(), by_day.descending()),
        (Column::Change1d, true)
    );
    let again = by_day.clone().sorted_by(Column::Change1d);
    assert_eq!(
        (again.sort(), again.descending()),
        (Column::Change1d, false)
    );
    assert_eq!(again.sorted_by(Column::Change1d), by_day);
    let by_name = by_day.sorted_by(Column::Name);
    assert_eq!(
        (by_name.sort(), by_name.descending()),
        (Column::Name, false)
    );
    assert_eq!(defaults.clone().sorted_by(Column::Rank), defaults);
    assert_eq!(
        serde_json::to_value(defaults.clone().sorted_by(Column::Tvl)).unwrap(),
        json!({ "reversed": true })
    );

    // What it does not know, or of the wrong type, is the default.
    let damaged = DefiProtocolsSettings::from(json!({
        "category": 7, "sort": "rank", "reversed": "yes",
    }));
    assert_eq!(damaged, defaults);
    assert_eq!(DefiProtocolsSettings::from(json!(null)), defaults);
    assert_eq!(defaults.clone().with_category(Some("  ")), defaults);
    assert!(
        Column::ALL
            .iter()
            .all(|column| Column::from_id(column.id()) == Some(*column))
    );
}

fn list() -> Protocols {
    Protocols::read(&json!({
        "total": 9086,
        "fetchedAt": LATEST_MS,
        "protocols": [
            { "slug": "binance-cex", "name": "Binance CEX", "symbol": "BNB", "category": "CEX",
              "chains": ["Ethereum", "Bitcoin", "Solana"], "tvl": 108_117_746_518.9,
              "change1h": -1.238, "change1d": 0.001, "change7d": 12.5 },
            { "slug": "lido", "name": "lido", "category": "Liquid Staking",
              "chains": ["Ethereum"], "tvl": 20_000_000_000.0, "change1d": 3.0,
              "mcap": 1_000_000_000.0 },
            { "slug": "aave-v3", "name": "Aave V3", "symbol": "AAVE",
              "tvl": 15_500_000_000.0, "change1d": -2.0, "mcap": 31_000_000_000.0 },
            { "slug": "hyperliquid-perps", "name": "Hyperliquid Perps", "category": "Derivatives" },
        ],
    }))
    .unwrap()
}

#[test]
fn rows_are_ordered_by_a_column_with_what_it_lacks_last() {
    let list = list();
    let slugs = |column: Column, descending: bool| -> Vec<&str> {
        session::order_of(&list, column, descending)
            .into_iter()
            .map(|place| list.protocols[place].slug.as_str())
            .collect()
    };
    // The list's own order is largest TVL first.
    assert_eq!(
        slugs(Column::Tvl, true),
        ["binance-cex", "lido", "aave-v3", "hyperliquid-perps"]
    );
    // The other way round, a row with no figure is still last.
    assert_eq!(
        slugs(Column::Tvl, false),
        ["aave-v3", "lido", "binance-cex", "hyperliquid-perps"]
    );
    assert_eq!(
        slugs(Column::Change1d, true),
        ["lido", "binance-cex", "aave-v3", "hyperliquid-perps"]
    );
    // Rows that tie (here, with no ratio) keep their order by TVL.
    assert_eq!(
        slugs(Column::McapTvl, true),
        ["aave-v3", "lido", "binance-cex", "hyperliquid-perps"]
    );
    // Names read regardless of case; categories with none last.
    assert_eq!(
        slugs(Column::Name, false),
        ["aave-v3", "binance-cex", "hyperliquid-perps", "lido"]
    );
    assert_eq!(
        slugs(Column::Category, false),
        ["binance-cex", "hyperliquid-perps", "lido", "aave-v3"]
    );
    assert_eq!(slugs(Column::Chains, true)[..2], ["binance-cex", "lido"]);

    // A table's row keeps its rank by TVL whatever the order shown.
    let table = Table::of(Some(std::rc::Rc::new(list.clone())), Column::Name, false);
    assert_eq!(table.len(), 4);
    let (rank, row) = table.row(0).unwrap();
    assert_eq!((rank, row.slug.as_str()), (3, "aave-v3"));
    assert!(table.row(4).is_none());
    assert_eq!(Table::default().len(), 0);
}

#[test]
fn cells_read_short_figures_and_tone_what_moved() {
    let list = list();
    let read = |at: usize, column: Column| {
        let (text, tone) = session::cell_of(at + 1, &list.protocols[at], column);
        (text, tone)
    };
    let cell = |text: &str, tone: Tone| (text.to_owned(), tone);
    assert_eq!(read(0, Column::Rank), cell("1", Tone::Faint));
    assert_eq!(read(0, Column::Name), cell("Binance CEX", Tone::Plain));
    assert_eq!(read(0, Column::Tvl), cell("$108.1B", Tone::Plain));
    assert_eq!(read(0, Column::Change1h), cell("-1.24%", Tone::Down));
    // What rounds to nothing has no color.
    assert_eq!(read(0, Column::Change1d), cell("0.00%", Tone::Muted));
    assert_eq!(read(0, Column::Change7d), cell("+12.50%", Tone::Up));
    assert_eq!(read(0, Column::Category), cell("CEX", Tone::Muted));
    assert_eq!(read(0, Column::Chains), cell("Ethereum +2", Tone::Muted));
    assert_eq!(read(1, Column::Chains), cell("Ethereum", Tone::Muted));
    assert_eq!(read(1, Column::Mcap), cell("$1.000B", Tone::Plain));
    assert_eq!(read(1, Column::McapTvl), cell("0.05", Tone::Plain));
    assert_eq!(read(2, Column::McapTvl), cell("2.00", Tone::Plain));
    // What a row lacks is a dash.
    for column in [
        Column::Tvl,
        Column::Change1h,
        Column::Chains,
        Column::Mcap,
        Column::McapTvl,
    ] {
        assert_eq!(read(3, column), cell(NONE, Tone::Faint), "{column:?}");
    }
    assert_eq!(read(2, Column::Category), cell(NONE, Tone::Faint));

    // The strip: what shows of what matches, and when it is from.
    let utc = ClockPrefs::default();
    assert_eq!(
        view::summary(&list, utc),
        "4 of 9,086 protocols · as of Nov 21 13:44"
    );
    let one = Protocols::read(&json!({ "protocols": [{ "slug": "a", "name": "A" }] })).unwrap();
    assert_eq!(view::summary(&one, utc), "1 protocol");
    assert_eq!(view::summary(&Protocols::default(), utc), "0 protocols");

    // How a list stands, and what is said of it.
    let failure = ApiError::network("offline");
    let stale = Protocols {
        stale: true,
        ..list.clone()
    };
    assert_eq!(Standing::of(None, None), Standing::Loading);
    assert_eq!(Standing::of(Some(&list), None), Standing::Ready);
    assert_eq!(Standing::of(Some(&stale), None), Standing::Stale);
    assert_eq!(
        Standing::of(None, Some(&failure)),
        Standing::Failed("offline".to_owned())
    );
    assert!(Standing::Loading.note().is_some() && Standing::Searching.note().is_some());
    assert!(Standing::Ready.note().is_none() && Standing::Ready.warning().is_none());
    assert!(Standing::Stale.warning().is_some());
    assert!(
        Standing::of(Some(&list), Some(&failure))
            .warning()
            .is_some_and(|warning| warning.contains("offline"))
    );
}

/// One listing every category.
#[component]
fn Listing() -> impl IntoElement {
    workspace_of(WidgetKind::DefiProtocols, None)
}

/// One saved listing a category, its names from A.
#[component]
fn Filtered() -> impl IntoElement {
    workspace_of(
        WidgetKind::DefiProtocols,
        Some(json!({ "category": "Derivatives", "sort": "name" })),
    )
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

/// The names of the rows shown, top to bottom.
fn names(app: &Headless) -> Vec<String> {
    const NAMES: [&str; 7] = [
        "Binance CEX",
        "Lido",
        "Aave V3",
        "Hyperliquid Bridge",
        "Jupiter Perpetual Exchange",
        "trade[XYZ]",
        "Hyperliquid Perps",
    ];
    let mut shown: Vec<(f32, String)> = app
        .screen()
        .layout_nodes()
        .into_iter()
        .filter_map(|node| {
            let text = node.text.as_ref()?.text.text.as_str();
            let name = NAMES.iter().find(|name| text.starts_with(**name))?;
            Some((node.bounds.origin.y.0, (*name).to_owned()))
        })
        .collect();
    shown.sort_by(|a, b| a.0.total_cmp(&b.0));
    shown.dedup_by(|a, b| a.1 == b.1);
    shown.into_iter().map(|(_, name)| name).collect()
}

#[test]
fn the_table_lists_protocols_by_tvl_and_orders_filters_and_searches_them() {
    let (mut app, network) = start(nsh_testing::windowed_root::<Listing>());

    // Every category, largest first, with what the strip says of them.
    app.wait_for("$108.1B").unwrap();
    for shown in [
        "Binance CEX\u{2002}BNB",
        "Lido\u{2002}LDO",
        "trade[XYZ]",
        "$20.00B",
        "Liquid Staking",
        "Ethereum +2",
        "-1.50%",
        "$950",
    ] {
        assert!(app.shows(shown), "{shown}");
    }
    assert!(
        app.screen()
            .matches("id:defi-protocols-summary >> text:7 of 9,086 protocols · as of Nov 21 13:44")
            .unwrap()
    );
    assert!(app.screen().matches("label:Open DefiLlama").unwrap());
    assert_eq!(
        names(&app),
        [
            "Binance CEX",
            "Lido",
            "Aave V3",
            "Hyperliquid Bridge",
            "Jupiter Perpetual Exchange",
            "trade[XYZ]",
            "Hyperliquid Perps",
        ]
    );
    assert_eq!(asked(&network), ["/v1/defillama/protocols?limit=500"]);

    // The titles sit side by side across the table, which is as wide as
    // its columns at their narrowest in a pane with no more room.
    let (table_left, table_width) = span(&app, "defi-protocols-table");
    let (_, pane_width) = span(&app, "defi-protocols");
    assert!(pane_width < 936.0, "the pane is {pane_width} wide");
    assert!((table_width - 936.0).abs() < 0.5, "{table_width}");
    let mut edge = table_left;
    for column in Column::ALL {
        let (left, width) = span(&app, &format!("defi-protocols-title-{}", column.id()));
        assert!(
            (left - edge).abs() < 0.5,
            "{column:?} at {left}, after {edge}"
        );
        edge = left + width;
    }
    assert!((edge - (table_left + table_width)).abs() < 0.5);

    // A title orders the rows by its column, and again the other way round.
    app.press("label:Sort by Name").unwrap();
    app.wait_for("Name ▴").unwrap();
    assert_eq!(names(&app)[..2], ["Aave V3", "Binance CEX"]);
    app.press("label:Sort by Name").unwrap();
    app.wait_for("Name ▾").unwrap();
    assert_eq!(names(&app)[..2], ["trade[XYZ]", "Lido"]);
    app.press("label:Sort by TVL").unwrap();
    app.wait_for("TVL ▾").unwrap();
    assert_eq!(names(&app)[0], "Binance CEX");

    // The select offers the categories the list named, and lists one.
    app.press("label:Category").unwrap();
    app.press("role:option[Derivatives]").unwrap();
    app.wait_for_gone("$108.1B", WAIT).unwrap();
    app.wait_for("id:defi-protocols-summary >> text:3 protocols · as of Nov 21 13:44")
        .unwrap();
    assert_eq!(
        names(&app),
        [
            "Jupiter Perpetual Exchange",
            "trade[XYZ]",
            "Hyperliquid Perps"
        ]
    );
    assert!(
        asked(&network)
            .contains(&"/v1/defillama/protocols?category=Derivatives&limit=500".to_owned()),
        "{:?}",
        asked(&network)
    );

    // A search asks the whole list of the category once typing has stopped.
    app.type_text("id:defi-protocols-search", "hyper").unwrap();
    app.wait_for_gone("trade[XYZ]", WAIT).unwrap();
    assert_eq!(names(&app), ["Hyperliquid Perps"]);
    assert!(
        asked(&network).contains(
            &"/v1/defillama/protocols?category=Derivatives&search=hyper&limit=500".to_owned()
        ),
        "{:?}",
        asked(&network)
    );
    app.clear("id:defi-protocols-search").unwrap();
    app.type_text("id:defi-protocols-search", "zzz").unwrap();
    app.wait_for("No protocols match.").unwrap();
    // Emptied, the category's list is back, with nothing asked again.
    let before = asked(&network).len();
    app.clear("id:defi-protocols-search").unwrap();
    app.wait_for("trade[XYZ]").unwrap();
    assert_eq!(asked(&network).len(), before);

    // In a pane wider than the columns the table fills it, the name's
    // column taking the room the others leave.
    assert!((span(&app, "defi-protocols-title-name").1 - 170.0).abs() < 0.5);
    app.set_viewport(Size::new(px(2600.0), px(800.0))).unwrap();
    app.wait(std::time::Duration::from_millis(200)).unwrap();
    let (_, pane_width) = span(&app, "defi-protocols");
    let (_, table_width) = span(&app, "defi-protocols-table");
    assert!(pane_width > 936.0, "the pane is {pane_width} wide");
    assert!(
        (table_width - pane_width).abs() < 0.5,
        "{table_width} in {pane_width}"
    );
    let name_width = span(&app, "defi-protocols-title-name").1;
    assert!((name_width - (170.0 + pane_width - 936.0)).abs() < 0.5);
}

#[test]
fn a_saved_category_and_order_are_listed_as_saved() {
    let (mut app, network) = start(nsh_testing::windowed_root::<Filtered>());
    app.wait_for("$567.1M").unwrap();
    app.wait_for("Name ▴").unwrap();
    assert_eq!(
        names(&app),
        [
            "Hyperliquid Perps",
            "Jupiter Perpetual Exchange",
            "trade[XYZ]"
        ]
    );
    // The category's list is what it browses; the whole list is asked
    // once, for the categories its select offers.
    app.press("label:Category").unwrap();
    app.wait_for("role:option[Liquid Staking]").unwrap();
    let requests = asked(&network);
    assert_eq!(requests.len(), 2, "{requests:?}");
    assert!(
        requests.contains(&"/v1/defillama/protocols?category=Derivatives&limit=500".to_owned())
    );
    assert!(requests.contains(&"/v1/defillama/protocols?limit=500".to_owned()));
}
