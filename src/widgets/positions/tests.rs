use std::time::Duration;

use nsh_core::{element::Root, prelude::*};
use nsh_testing::WAIT;
use serde_json::json;

use super::{session, settings::PositionsSettings};
use crate::{
    data::{
        portfolio::{Account, Direction, Leverage, MarginMode, Position},
        Portfolio,
    },
    widgets::{
        common::accounts::{
            testing::{
                account, accounts, asked, asked_lighter, count, lighter_account, on, start,
                start_manual, workspace_of, workspace_within, IDLE, LIMITED, TRADER, USER,
            },
            AccountSettings, Mark, Tone, NONE,
        },
        WidgetKind,
    },
};

#[test]
fn settings_save_the_account_alone_and_read_leniently() {
    // Defaults save nothing.
    let defaults = PositionsSettings::default();
    assert_eq!(serde_json::to_value(&defaults).unwrap(), json!({}));
    assert!(defaults.accounts().is_empty());

    // The account, as an object of its venue and address.
    let watched = Account {
        venue: "hyperliquid".to_owned(),
        address: USER.to_owned(),
    };
    let settings = defaults.clone().with_accounts([watched.clone()]);
    assert_eq!(serde_json::to_value(&settings).unwrap(), account(USER));
    assert_eq!(PositionsSettings::from(account(USER)), settings);
    assert_eq!(settings.accounts(), std::slice::from_ref(&watched));
    // Several, on one exchange or on several, as a list.
    let both = accounts(&[on("hyperliquid", USER), on("lighterxyz", TRADER)]);
    let several = PositionsSettings::from(both.clone());
    assert_eq!(several.accounts().len(), 2);
    assert_eq!(serde_json::to_value(&several).unwrap(), both);
    assert_eq!(settings.with_accounts([]), defaults);

    // Anything else reads as no account.
    for damaged in [
        json!(null),
        json!([]),
        json!({ "account": USER }),
        json!({ "account": { "venue": "hyperliquid" } }),
    ] {
        assert_eq!(
            PositionsSettings::from(damaged.clone()),
            defaults,
            "{damaged}"
        );
    }
}

fn position(instrument: &str, value: Option<f64>) -> Position {
    Position {
        instrument: instrument.to_owned(),
        direction: Direction::Long,
        size: 2.0,
        entry_price: None,
        mark_price: None,
        value,
        unrealized_pnl: None,
        return_on_equity: None,
        liquidation_price: None,
        margin_used: None,
        leverage: None,
    }
}

#[test]
fn rows_say_each_position_the_largest_first() {
    let short = Position {
        direction: Direction::Short,
        size: 0.1,
        entry_price: Some(60_000.0),
        mark_price: Some(61_000.0),
        unrealized_pnl: Some(-100.0),
        return_on_equity: Some(-0.0833),
        margin_used: Some(1_220.0),
        leverage: Some(Leverage {
            value: 5.0,
            mode: MarginMode::Cross,
        }),
        ..position("BTC", Some(6_100.0))
    };
    let rows = session::rows_of(&[
        position("ZRO", None),
        position("eth", Some(100.0)),
        short,
        position("ARB", Some(100.0)),
    ]);
    // By worth, then by name; one of unknown worth last.
    let order: Vec<&str> = rows.iter().map(|row| row.instrument.as_str()).collect();
    assert_eq!(order, ["BTC", "ARB", "eth", "ZRO"]);

    let btc = &rows[0];
    assert_eq!(btc.direction, Direction::Short);
    assert_eq!(btc.leverage, "5x Cross");
    assert_eq!(btc.size, "0.1");
    assert_eq!(btc.value, "$6,100.00");
    assert_eq!(
        (btc.entry.as_str(), btc.mark.as_str()),
        ("60000.0", "61000.0")
    );
    assert_eq!(btc.pnl, "-$100.00 (-8.33%)");
    assert_eq!(btc.tone, Some(Tone::Loss));
    assert_eq!(btc.liquidation, NONE);
    assert_eq!(btc.margin, "$1,220.00");

    // What is not reported is a dash, and has no tone.
    let zro = &rows[3];
    assert_eq!(zro.leverage, "");
    assert_eq!(
        [&zro.value, &zro.entry, &zro.mark, &zro.pnl, &zro.margin].map(String::as_str),
        [NONE; 5]
    );
    assert_eq!(zro.tone, None);

    // A profit says so.
    let gain = Position {
        unrealized_pnl: Some(12.5),
        ..position("SOL", Some(1.0))
    };
    assert_eq!(session::rows_of(&[gain])[0].pnl, "+$12.50");

    // A long and a short of one instrument are two rows.
    let hedge = Position {
        direction: Direction::Short,
        ..position("ETH", Some(1.0))
    };
    let both = session::rows_of(&[position("ETH", Some(1.0)), hedge]);
    assert_ne!(both[0].key, both[1].key);
    // One account's rows say nothing of whose they are.
    assert!(both.iter().all(|row| row.owner.is_none()));
}

#[test]
fn several_accounts_positions_are_one_table_each_marked_with_its_account() {
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
    let (first, second) = (mark("hyperliquid"), mark("lighterxyz"));
    let held = [position("BTC", Some(500.0)), position("ETH", Some(50.0))];
    // An address's two accounts on one exchange may hold a market twice.
    let twice = [position("BTC", Some(900.0)), position("BTC", Some(100.0))];
    let rows = session::rows_of_all(&[
        (Some(first.clone()), &held[..]),
        (Some(second.clone()), &twice[..]),
    ]);
    // The largest first, whichever account each is of.
    let listed: Vec<(&str, &str, &str)> = rows
        .iter()
        .map(|row| {
            (
                row.owner.as_ref().unwrap().named.as_str(),
                row.instrument.as_str(),
                row.value.as_str(),
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            ("Lighter 0x0123…4567", "BTC", "$900.00"),
            ("Hyperliquid 0x0123…4567", "BTC", "$500.00"),
            ("Lighter 0x0123…4567", "BTC", "$100.00"),
            ("Hyperliquid 0x0123…4567", "ETH", "$50.00"),
        ]
    );
    // Every row is told apart.
    for (at, row) in rows.iter().enumerate() {
        assert!(rows[at + 1..].iter().all(|other| other.key != row.key));
    }
}

/// A workspace of one Positions widget watching no account.
#[component]
fn Unset() -> impl IntoElement {
    workspace_of(WidgetKind::Positions, None)
}

/// One watching the account Hyperliquid refuses.
#[component]
fn Refused() -> impl IntoElement {
    workspace_of(WidgetKind::Positions, Some(account(LIMITED)))
}

#[test]
fn the_table_shows_the_account_s_positions_and_follows_its_settings() {
    let (mut app, network) = start(Root::of::<Unset>());

    // No account: the widget asks for one, and asks Hyperliquid nothing.
    app.wait_for("id:positions-no-account").unwrap();
    assert!(
        !app.screen().matches("id:positions-table").unwrap(),
        "no table without an account"
    );
    assert!(asked(&network).is_empty());

    // What is no address is said so, and saved as nothing.
    app.type_text("id:positions-address", "vitalik.eth\n")
        .unwrap();
    app.wait_for("id:positions-address-problem").unwrap();
    assert!(app.shows("Enter the account's address: 0x followed by 40 hexadecimal characters."));
    assert!(asked(&network).is_empty());

    // An address, however it is typed: its positions arrive, the larger
    // first, and the account is named in the strip.
    app.clear("id:positions-address").unwrap();
    app.type_text(
        "id:positions-address",
        &format!(" {}\n", USER.to_uppercase().replace("0X", "0x")),
    )
    .unwrap();
    app.wait_for("$6,100.00").unwrap();
    assert!(!app.screen().matches("id:positions-no-account").unwrap());
    assert!(app
        .screen()
        .matches("label:Change account: Hyperliquid 0x0123…4567")
        .unwrap());
    // (Titles and directions are drawn in capitals.)
    for shown in [
        "MARKET",
        "PNL (ROE)",
        "BTC",
        "SHORT",
        "5x Cross",
        "0.1",
        "60000.0",
        "61000.0",
        "+$100.00 (+8.33%)",
        "$1,220.00",
        "ETH",
        "LONG",
        "20x Isolated",
        "0.0335",
        "$100.03",
        "2986.30",
        "-$0.01 (-0.27%)",
        "2866.27",
        "$4.97",
    ] {
        assert!(app.shows(shown), "{shown}");
    }
    let texts = app.screen().texts();
    let at = |text: &str| texts.iter().position(|shown| *shown == text).unwrap();
    assert!(at("BTC") < at("ETH"), "the position worth more is first");
    assert!(app
        .screen()
        .in_selectable_view("id:positions-table")
        .unwrap());
    assert!(!app.shows("No open positions"));

    // The account's perpetuals alone are asked for, by its address in
    // lower case.
    let requests = asked(&network);
    assert!(count(&requests, "clearinghouseState", USER) >= 1);
    assert_eq!(count(&requests, "clearinghouseState", USER), requests.len());

    // The strip opens the settings, which list the account as saved, the
    // field empty for another; × takes it out, and one with nothing open
    // says so in its place.
    app.press("id:positions-account").unwrap();
    app.wait_for("id:positions-settings").unwrap();
    const FIELD: &str = "id:positions-settings >> id:positions-address";
    assert!(app
        .screen()
        .matches("id:positions-settings >> id:positions-accounts >> text:0x0123…4567")
        .unwrap());
    assert_eq!(app.screen().input_text(FIELD).unwrap(), Some(""));
    app.press("id:positions-settings >> id:positions-remove-0")
        .unwrap();
    app.wait_for("id:positions-no-account").unwrap();
    app.type_text(FIELD, &format!("{IDLE}\n")).unwrap();
    app.wait_for("No open positions").unwrap();
    assert!(!app.shows("BTC") && !app.shows("ETH"));
    assert!(count(&asked(&network), "clearinghouseState", IDLE) >= 1);

    // With none left it watches nothing again.
    app.press("id:positions-settings >> id:positions-remove-0")
        .unwrap();
    app.wait_for("id:positions-no-account").unwrap();
    app.press("label:Close Positions settings").unwrap();
    app.wait_for_gone("id:positions-settings", WAIT).unwrap();
    assert!(!app.shows("No open positions"));
}

#[test]
fn a_request_hyperliquid_refuses_is_said_and_not_shown_as_an_empty_account() {
    let (mut app, _network) = start(Root::of::<Refused>());
    app.wait_for(
        "Could not load positions: Hyperliquid's request limit for this connection was \
         reached; requests resume within a minute",
    )
    .unwrap();
    assert!(!app.shows("No open positions"));
    assert!(app
        .screen()
        .matches("label:Change account: Hyperliquid 0x0000…00ff")
        .unwrap());
}

/// A workspace of one Positions widget watching the stubbed account, with
/// a budget of two of its requests a minute.
#[component]
fn Paced() -> impl IntoElement {
    workspace_within(WidgetKind::Positions, Some(account(USER)), 4)
}

#[test]
fn the_account_is_asked_for_every_five_minutes_within_a_small_budget() {
    let (mut app, network) = start_manual(Root::of::<Paced>());
    let polls = || count(&asked(&network), "clearinghouseState", USER);
    app.wait_for("$6,100.00").unwrap();
    assert_eq!(polls(), 1);

    // Nothing more for five minutes.
    app.run_for(Duration::from_secs(4 * 60)).unwrap();
    assert_eq!(polls(), 1);

    // Then once more, which the budget (two requests a minute) affords,
    // and nothing failed meanwhile: the positions stay, with no warning
    // over them.
    app.run_for(Duration::from_secs(2 * 60)).unwrap();
    assert_eq!(polls(), 2);
    assert!(app.shows("$6,100.00"));
    assert!(!app
        .screen()
        .texts()
        .iter()
        .any(|text| text.starts_with("Not up to date") || text.starts_with("Could not load")));
}

/// One watching an address with two accounts on Lighter.
#[component]
fn Trading() -> impl IntoElement {
    workspace_of(WidgetKind::Positions, Some(lighter_account(TRADER)))
}

#[test]
fn a_lighter_address_lists_the_positions_of_its_accounts() {
    let (mut app, network) = start(Root::of::<Trading>());
    app.wait_for("$31,000.00").unwrap();
    assert!(app
        .screen()
        .matches("label:Change account: Lighter 0xe117…b9a8")
        .unwrap());
    for shown in [
        // Its main account's BTC long, at 5% of its worth: 20x.
        "20x Cross",
        "0.5",
        "60000.0",
        "62000.0",
        "+$1,000.00 (+66.67%)",
        "30000.5",
        "$1,550.00",
        // Its isolated SOL short, which holds what is set aside for it.
        "SOL",
        "10x Isolated",
        "$6,500.00",
        "+$500.00 (+71.43%)",
        "120.250",
        "$650.00",
        // Its sub-account's BTC short.
        "0.01",
        "$620.00",
        "-$10.00 (-32.79%)",
        "$31.00",
    ] {
        assert!(app.shows(shown), "{shown}");
    }
    // The market it holds nothing of is no row, and neither is the pool's.
    assert!(!app.shows("POL") && !app.shows("$560,000.00"));
    let texts = app.screen().texts();
    assert_eq!(texts.iter().filter(|text| **text == "BTC").count(), 2);
    // One account shown: no row says whose it is.
    assert_eq!(app.screen().count("label:Lighter 0xe117…b9a8").unwrap(), 0);

    // Its accounts were asked for once, and nothing else.
    let requests = asked_lighter(&network);
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert!(requests[0].starts_with("/api/v1/account?by=l1_address&value=0xe117"));
    assert!(
        asked(&network).is_empty(),
        "nothing is asked of Hyperliquid"
    );
}

/// One watching an account on Hyperliquid and an address on Lighter.
#[component]
fn Everywhere() -> impl IntoElement {
    let saved = accounts(&[on("hyperliquid", USER), on("lighterxyz", TRADER)]);
    workspace_of(WidgetKind::Positions, Some(saved))
}

#[test]
fn accounts_on_several_exchanges_are_one_table() {
    let (mut app, _network) = start(Root::of::<Everywhere>());
    // Both exchanges' positions, the largest first.
    app.wait_for("$31,000.00").unwrap();
    app.wait_for("$6,100.00").unwrap();
    for shown in [
        "$6,500.00",
        "$620.00",
        "$100.03",
        "20x Isolated",
        "10x Isolated",
    ] {
        assert!(app.shows(shown), "{shown}");
    }
    let texts = app.screen().texts();
    let at = |text: &str| texts.iter().position(|shown| *shown == text).unwrap();
    assert!(at("$31,000.00") < at("$6,500.00") && at("$6,500.00") < at("$6,100.00"));
    assert!(at("$6,100.00") < at("$620.00") && at("$620.00") < at("$100.03"));

    // The strip counts the accounts, and each row says whose it is.
    assert!(app
        .screen()
        .matches("label:Change accounts: Hyperliquid 0x0123…4567, Lighter 0xe117…b9a8")
        .unwrap());
    assert!(app
        .screen()
        .matches("id:positions-account >> text:2 accounts")
        .unwrap());
    assert_eq!(
        app.screen()
            .count("id:positions-table >> label:Lighter 0xe117…b9a8")
            .unwrap(),
        3
    );
    assert_eq!(
        app.screen()
            .count("id:positions-table >> label:Hyperliquid 0x0123…4567")
            .unwrap(),
        2
    );

    // Taking one out leaves the other's, unmarked.
    app.press("id:positions-account").unwrap();
    app.wait_for("id:positions-settings").unwrap();
    app.press("id:positions-settings >> id:positions-remove-1")
        .unwrap();
    app.wait_for_gone("$31,000.00", WAIT).unwrap();
    assert!(app.shows("$6,100.00"));
    assert_eq!(
        app.screen()
            .count("id:positions-table >> label:Hyperliquid 0x0123…4567")
            .unwrap(),
        0
    );
}

/// One watching an account on Hyperliquid and one Lighter refuses.
#[component]
fn Halved() -> impl IntoElement {
    let saved = accounts(&[on("hyperliquid", USER), on("lighterxyz", LIMITED)]);
    workspace_of(WidgetKind::Positions, Some(saved))
}

#[test]
fn an_account_that_cannot_be_read_is_named_over_the_others_positions() {
    let (mut app, _network) = start(Root::of::<Halved>());
    app.wait_for(
        "Not every account is shown. Lighter 0x0000…00ff: Lighter's request limit for this \
         connection was reached; requests resume within a minute",
    )
    .unwrap();
    assert!(app.shows("$6,100.00") && app.shows("$100.03"));
    assert!(!app.shows("No open positions"));
}
