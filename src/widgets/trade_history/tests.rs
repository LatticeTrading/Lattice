use nsh_core::{element::Root, prelude::*};
use serde_json::json;

use super::{
    session::{self, Totals, MAX_ROWS},
    settings::TradeHistorySettings,
    view::when,
};
use crate::{
    data::{
        portfolio::{Account, Fill, Fills, TradeSide},
        Portfolio,
    },
    shell::clock::ClockPrefs,
    widgets::{
        common::accounts::{
            testing::{
                account, accounts, asked, asked_lighter, asked_txflow, count, lighter_account, on,
                start, txflow_account, workspace_of, IDLE, TRADER, TRADER_MAIN, TRADER_SUB, USER,
            },
            AccountSettings, Mark, Tone, NONE,
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
    let settings = defaults.clone().with_accounts([watched.clone()]);
    assert_eq!(settings.accounts(), std::slice::from_ref(&watched));
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

    // Two accounts' trades are one list, each marked with its account: a
    // trade both were sides of is a row of each, and the totals count both.
    let portfolio = Portfolio::new([]);
    let mark = |venue: &str| {
        Mark::of(
            &portfolio,
            &Account {
                venue: venue.to_owned(),
                address: USER.to_owned(),
            },
        )
    };
    let other = Fills(vec![
        fill("3", 2_000.0, Some((0.02, "USDC")), None),
        fill("9", 5_000.0, None, Some(1.5)),
    ]);
    let rows = session::rows_of_all(&[
        (Some(mark("hyperliquid")), &fills),
        (Some(mark("lighterxyz")), &other),
    ]);
    let listed: Vec<(&str, f64)> = rows
        .iter()
        .map(|row| (row.owner.as_ref().unwrap().named.as_str(), row.time_ms))
        .collect();
    assert_eq!(
        listed,
        [
            ("Lighter 0x0123…4567", 5_000.0),
            ("Hyperliquid 0x0123…4567", 3_000.0),
            ("Hyperliquid 0x0123…4567", 2_000.0),
            ("Lighter 0x0123…4567", 2_000.0),
            ("Hyperliquid 0x0123…4567", 1_000.0),
        ]
    );
    assert_ne!(rows[2].key, rows[3].key);
    let totals = Totals::of_all(Some(&[&fills, &other]));
    assert_eq!((totals.count, totals.since_ms), (6, Some(1_000.0)));
    assert_eq!(
        [&totals.volume, &totals.fees, &totals.closed_pnl].map(String::as_str),
        ["$600.00", "$0.02", "+$7.50"]
    );
    assert!(session::rows_of(&fills)
        .iter()
        .all(|row| row.owner.is_none()));

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

/// One watching an address with two accounts on Lighter.
#[component]
fn Trading() -> impl IntoElement {
    workspace_of(WidgetKind::TradeHistory, Some(lighter_account(TRADER)))
}

#[test]
fn a_lighter_address_lists_the_trades_of_its_accounts() {
    let (mut app, network) = start(Root::of::<Trading>());
    app.wait_for("$30,000.00").unwrap();
    for shown in [
        // Its main account bought BTC, taking, at 0.028%.
        "Mar 13 21:06:40",
        "BUY",
        "Taker",
        "60000.0",
        "0.5",
        "$8.4",
        // It sold ETH, making, at 0.002%, and closed a long with it.
        "ETH",
        "SELL",
        "Maker",
        "1800.00",
        "$1,800.00",
        "$0.036",
        "+$25.50",
        // It sold BTC to its own sub-account: a trade of each.
        "61000.0",
        "$610.00",
        // The totals.
        "4 trades since Mar 13 21:04:40",
        "$33,020.00",
        "$8.44",
    ] {
        assert!(app.shows(shown), "{shown}");
    }
    let texts = app.screen().texts();
    assert_eq!(texts.iter().filter(|text| **text == "$610.00").count(), 2);
    assert!(app
        .screen()
        .matches("id:trade-history-closed-pnl >> text:+$25.50")
        .unwrap());

    // Its accounts once, then the latest trades of each.
    let requests = asked_lighter(&network);
    assert_eq!(requests.len(), 3, "{requests:?}");
    assert!(requests[0].starts_with("/api/v1/account?by=l1_address&value=0xe117"));
    for index in [TRADER_MAIN, TRADER_SUB] {
        let asked = format!("/api/v1/trades?sort_by=timestamp&account_index={index}&limit=100");
        assert!(requests.contains(&asked), "{asked}");
    }
    assert!(
        asked(&network).is_empty(),
        "nothing is asked of Hyperliquid"
    );
}

/// One watching an account on Hyperliquid and an address on Lighter.
#[component]
fn Everywhere() -> impl IntoElement {
    let saved = accounts(&[on("hyperliquid", USER), on("lighterxyz", TRADER)]);
    workspace_of(WidgetKind::TradeHistory, Some(saved))
}

#[test]
fn accounts_on_several_exchanges_are_one_list_of_trades() {
    let (mut app, _network) = start(Root::of::<Everywhere>());
    app.wait_for("$30,000.00").unwrap();
    app.wait_for("CLOSE SHORT").unwrap();
    // The newest first, whichever exchange each is on; the totals are of
    // all of them.
    let texts = app.screen().texts();
    let at = |text: &str| texts.iter().position(|shown| *shown == text).unwrap();
    assert!(at("$30,000.00") < at("$2,950.00"));
    assert!(app.shows("6 trades since Feb 22 17:13:20"));
    assert!(app
        .screen()
        .matches("id:trade-history-volume >> text:$36,070.04")
        .unwrap());
    // Each row says whose it is.
    assert_eq!(
        app.screen()
            .count("id:trade-history-table >> label:Lighter 0xe117…b9a8")
            .unwrap(),
        4
    );
    assert_eq!(
        app.screen()
            .count("id:trade-history-table >> label:Hyperliquid 0x0123…4567")
            .unwrap(),
        2
    );
    assert!(app
        .screen()
        .matches("id:trade-history-account >> text:2 accounts")
        .unwrap());
}

/// One watching an address on TxFlow.
#[component]
fn Flowing() -> impl IntoElement {
    workspace_of(WidgetKind::TradeHistory, Some(txflow_account(USER)))
}

#[test]
fn a_txflow_address_lists_its_trades_with_what_each_did() {
    let (mut app, network) = start(Root::of::<Flowing>());
    app.wait_for("$30,000.00").unwrap();
    assert!(app
        .screen()
        .matches("label:Change account: TxFlow 0x0123…4567")
        .unwrap());
    for shown in [
        // TxFlow says only the side: what each did is read from the
        // position it started from.
        "OPEN LONG",
        "Taker",
        "60000.0",
        "0.5",
        "$8.4",
        "ETH",
        "CLOSE LONG",
        "Maker",
        "1800.00",
        "$1,800.00",
        "+$25.50",
        "2 trades since Mar 13 21:04:40",
        "$31,800.00",
    ] {
        assert!(app.shows(shown), "{shown}");
    }

    // The latest trades, once.
    let requests = asked_txflow(&network);
    assert_eq!(
        requests,
        [json!({ "type": "userFills", "user": USER, "limit": 2000 })]
    );
    assert!(asked(&network).is_empty() && asked_lighter(&network).is_empty());
}
