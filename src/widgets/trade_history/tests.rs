use nsh_core::{element::Root, prelude::*};
use serde_json::json;

use super::{
    session::{self, Totals, MAX_ROWS},
    settings::TradeHistorySettings,
    view::when,
};
use crate::{
    data::portfolio::{Account, Fill, Fills, TradeSide},
    shell::clock::ClockPrefs,
    widgets::{
        common::accounts::{
            testing::{account, asked, count, start, workspace_of, IDLE, USER},
            AccountSettings, Tone, NONE,
        },
        WidgetKind,
    },
};

#[test]
fn settings_save_the_account_alone_and_read_leniently() {
    let defaults = TradeHistorySettings::default();
    assert_eq!(serde_json::to_value(&defaults).unwrap(), json!({}));
    let watched = Account {
        venue: "hyperliquid".to_owned(),
        address: USER.to_owned(),
    };
    let settings = defaults.clone().with_account(Some(watched.clone()));
    assert_eq!(serde_json::to_value(&settings).unwrap(), account(USER));
    assert_eq!(TradeHistorySettings::from(account(USER)), settings);
    assert_eq!(
        TradeHistorySettings::from(json!({ "account": 7 })),
        defaults
    );
}

fn fill(id: &str, time_ms: f64, fee: Option<(f64, &str)>, closed_pnl: Option<f64>) -> Fill {
    Fill {
        id: id.to_owned(),
        time_ms,
        instrument: "HYPE".to_owned(),
        side: TradeSide::Sell,
        action: None,
        price: 40.0,
        size: 2.5,
        fee: fee.map(|(fee, _)| fee),
        fee_asset: fee.map(|(_, asset)| asset.to_owned()),
        closed_pnl,
        taker: None,
    }
}

#[test]
fn rows_list_the_latest_trades_once_and_totals_count_them_all() {
    let fills = Fills(vec![
        fill("1", 1_000.0, Some((0.0123, "USDC")), Some(-4.0)),
        fill("2", 3_000.0, Some((0.5, "HYPE")), None),
        fill("3", 2_000.0, Some((-0.01, "USDC")), Some(10.0)),
        fill("3", 2_000.0, None, None),
    ]);
    let rows = session::rows_of(&fills);
    let keys: Vec<&str> = rows.iter().map(|row| &*row.key).collect();
    assert_eq!(keys, ["2", "3", "1"], "newest first, a trade once");
    let (hype, rebate, paid) = (&rows[0], &rows[1], &rows[2]);
    // No action reported: its side; no liquidity: nothing.
    assert_eq!((hype.action.as_str(), hype.liquidity), ("Sell", ""));
    assert_eq!(
        [&hype.price, &hype.size, &hype.value, &hype.fee, &hype.pnl].map(String::as_str),
        ["40.000", "2.5", "$100.00", "0.5 HYPE", NONE]
    );
    assert_eq!(
        (rebate.fee.as_str(), rebate.pnl.as_str()),
        ("-$0.01", "+$10.00")
    );
    assert_eq!(rebate.tone, Some(Tone::Gain));
    assert_eq!(paid.fee, "$0.0123");

    // Fees in dollars are added up, the rest are not.
    let totals = Totals::of(Some(&fills));
    assert_eq!((totals.count, totals.listed), (4, 4));
    assert_eq!(totals.since_ms, Some(1_000.0));
    assert_eq!(
        [&totals.volume, &totals.fees, &totals.closed_pnl].map(String::as_str),
        ["$400.00", "$0.00", "+$6.00"]
    );
    assert_eq!(Totals::of(None).volume, NONE);

    // No more than the latest MAX_ROWS are listed.
    let many = Fills(
        (0..MAX_ROWS + 50)
            .map(|at| fill(&at.to_string(), at as f64, None, None))
            .collect(),
    );
    let rows = session::rows_of(&many);
    assert_eq!(rows.len(), MAX_ROWS);
    assert_eq!(*rows[0].key, *(MAX_ROWS + 49).to_string());
    let totals = Totals::of(Some(&many));
    assert_eq!((totals.count, totals.listed), (MAX_ROWS + 50, MAX_ROWS));

    // A time as the clock writes it.
    assert_eq!(
        when(1_708_622_398_623.0, ClockPrefs::default()),
        "Feb 22 17:19:58"
    );
}

/// A workspace of one Trade History watching the stubbed account.
#[component]
fn Watching() -> impl IntoElement {
    workspace_of(WidgetKind::TradeHistory, Some(account(USER)))
}

/// One watching an account that never traded.
#[component]
fn Idle() -> impl IntoElement {
    workspace_of(WidgetKind::TradeHistory, Some(account(IDLE)))
}

#[test]
fn the_table_shows_the_account_s_trades_newest_first() {
    let (mut app, network) = start(Root::of::<Watching>());
    app.wait_for("CLOSE SHORT").unwrap();
    // (Titles and actions are drawn in capitals.)
    for shown in [
        "TIME",
        "CLOSED PNL",
        "Feb 22 17:19:58",
        "BTC",
        "Maker",
        "59000.0",
        "0.05",
        "$2,950.00",
        "-$0.295",
        "+$50.00",
        "ETH",
        "OPEN LONG",
        "Taker",
        "2986.30",
        "$100.04",
        "$0.045",
        // The totals.
        "2 trades since Feb 22 17:13:20",
        "$3,050.04",
        "-$0.25",
    ] {
        assert!(app.shows(shown), "{shown}");
    }
    let texts = app.screen().texts();
    let at = |text: &str| texts.iter().position(|shown| *shown == text).unwrap();
    assert!(at("BTC") < at("ETH"), "the newest first");
    assert!(!app.shows("No trades"));
    for id in ["table", "totals"] {
        let selector = format!("id:trade-history-{id}");
        assert!(app.screen().in_selectable_view(&selector).unwrap(), "{id}");
    }

    // The trades alone are asked for, by time.
    let requests = asked(&network);
    assert!(count(&requests, "userFills", USER) >= 1);
    assert_eq!(count(&requests, "userFills", USER), requests.len());
    assert!(requests.iter().all(|body| body["aggregateByTime"] == true));
}

#[test]
fn an_account_that_never_traded_says_so() {
    let (mut app, _network) = start(Root::of::<Idle>());
    app.wait_for("No trades").unwrap();
    assert!(app.shows("$0.00"));
    assert!(!app.shows("0 trades"), "no count of nothing");
}
