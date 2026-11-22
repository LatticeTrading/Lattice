use std::{
    collections::{BTreeMap, HashMap},
    rc::Rc,
    time::{Duration, Instant},
};

use nsh_core::{element::Root, prelude::*, time::Clock};
use nsh_testing::{Headless, WAIT};
use serde_json::json;

use super::{
    session::{self, Close, Desk, Part, Totals, closing_order, exits_on},
    settings::PositionsSettings,
};
use crate::{
    data::{
        Portfolio,
        portfolio::{Account, Direction, Leverage, MarginMode, Position},
        prices::PriceKind,
        trading::{
            Exits, Instrument, Instruments, MARKET_SLIPPAGE, OpenOrder, OpenOrders, OrderKind,
            OrderRequest, Side,
        },
    },
    widgets::{
        WidgetKind,
        common::{
            accounts::{
                AccountSettings, Mark, NONE, Tone,
                testing::{
                    IDLE, LIMITED, SLOW, SLOW_FOR, TRADER, USER, account, accounts, asked,
                    asked_lighter, asked_txflow, count, lighter_account, on, start, start_manual,
                    txflow_account, workspace_of, workspace_within,
                },
            },
            trade_bench::{self, ACCOUNT, Bench, Exchange, acted, sent, start_on},
        },
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

    // The price the positions are valued at: the live one while left out,
    // the exchange's mark when asked for.
    assert_eq!(defaults.price(), PriceKind::Live);
    let marked = defaults.clone().with_price(PriceKind::Mark);
    assert_eq!(
        serde_json::to_value(&marked).unwrap(),
        json!({ "price": "mark" })
    );
    assert_eq!(PositionsSettings::from(json!({ "price": "mark" })), marked);
    assert_eq!(marked.with_price(PriceKind::Live), defaults);
    // With an account, beside it.
    let both = PositionsSettings::from(account(USER)).with_price(PriceKind::Mark);
    let mut saved = account(USER);
    saved["price"] = json!("mark");
    assert_eq!(serde_json::to_value(&both).unwrap(), saved);
    assert_eq!(PositionsSettings::from(saved), both);
    // What names neither is the live one.
    for damaged in [json!("live"), json!("last"), json!(5), json!(null)] {
        let read = PositionsSettings::from(json!({ "price": damaged }));
        assert_eq!(read, defaults, "{damaged}");
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
    let part = |mark: &Rc<Mark>, positions| Part {
        mark: Some(mark.clone()),
        positions,
        prices: None,
        desk: None,
    };
    let rows = session::rows_of_all(&[part(&first, &held[..]), part(&second, &twice[..])]);
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
    assert!(
        app.screen()
            .matches("label:Change account: Hyperliquid 0x0123…4567")
            .unwrap()
    );
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
    assert!(
        app.screen()
            .in_selectable_view("id:positions-table")
            .unwrap()
    );
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
    assert!(
        app.screen()
            .matches("id:positions-settings >> id:positions-accounts >> text:0x0123…4567")
            .unwrap()
    );
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
    assert!(
        app.screen()
            .matches("label:Change account: Hyperliquid 0x0000…00ff")
            .unwrap()
    );
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
    assert!(
        !app.screen()
            .texts()
            .iter()
            .any(|text| text.starts_with("Not up to date") || text.starts_with("Could not load"))
    );
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
    assert!(
        app.screen()
            .matches("label:Change account: Lighter 0xe117…b9a8")
            .unwrap()
    );
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

/// One watching an address on TxFlow.
#[component]
fn Flowing() -> impl IntoElement {
    workspace_of(WidgetKind::Positions, Some(txflow_account(USER)))
}

#[test]
fn a_txflow_address_lists_its_perpetuals_positions() {
    let (mut app, network) = start(Root::of::<Flowing>());
    app.wait_for("$31,000.00").unwrap();
    assert!(
        app.screen()
            .matches("label:Change account: TxFlow 0x0123…4567")
            .unwrap()
    );
    // Its isolated BTC long at TxFlow's own mark price, its return read
    // from TxFlow's percent.
    for shown in [
        "BTC-USDC",
        "10x Isolated",
        "0.5",
        "60000.0",
        "62000.0",
        "+$1,000.00 (+32.26%)",
        "54321.0",
        "$3,100.00",
    ] {
        assert!(app.shows(shown), "{shown}");
    }

    // Its perpetuals sub-account, once.
    let requests = asked_txflow(&network);
    assert_eq!(
        requests,
        [json!({ "type": "clearinghouseState", "user": USER, "accountType": "perp" })]
    );
    assert!(asked(&network).is_empty() && asked_lighter(&network).is_empty());
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
    assert!(
        app.screen()
            .matches("label:Change accounts: Hyperliquid 0x0123…4567, Lighter 0xe117…b9a8")
            .unwrap()
    );
    assert!(
        app.screen()
            .matches("id:positions-account >> text:2 accounts")
            .unwrap()
    );
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

/// One watching an account Hyperliquid is slow to answer about, and the
/// stubbed account.
#[component]
fn Staggered() -> impl IntoElement {
    let saved = accounts(&[on("hyperliquid", SLOW), on("hyperliquid", USER)]);
    workspace_of(WidgetKind::Positions, Some(saved))
}

/// One watching the slow account and one that never traded.
#[component]
fn Waiting() -> impl IntoElement {
    let saved = accounts(&[on("hyperliquid", SLOW), on("hyperliquid", IDLE)]);
    workspace_of(WidgetKind::Positions, Some(saved))
}

#[test]
fn an_account_shows_as_soon_as_it_arrives_without_waiting_for_the_others() {
    let (mut app, _network) = start_manual(Root::of::<Staggered>());
    // The account that has arrived is listed, under how many show so far.
    app.wait_for("$6,100.00").unwrap();
    app.wait_for("Loading positions… 1 of 2 accounts shown.")
        .unwrap();
    assert!(!app.shows("$25,931,382.67"));

    // The other joins it when it arrives, and nothing more is said.
    app.run_for(SLOW_FOR + Duration::from_secs(1)).unwrap();
    app.wait_for("$25,931,382.67").unwrap();
    assert!(app.shows("$6,100.00"));
    let loading = |app: &nsh_testing::Headless| {
        app.screen()
            .texts()
            .iter()
            .any(|text| text.starts_with("Loading positions"))
    };
    assert!(!loading(&app));

    // An account with nothing open does not say so for the rest: the one
    // still on its way may have some.
    let (mut app, _network) = start_manual(Root::of::<Waiting>());
    app.wait_for("Loading positions… 1 of 2 accounts shown.")
        .unwrap();
    assert!(!app.shows("No open positions"));
    app.run_for(SLOW_FOR + Duration::from_secs(1)).unwrap();
    app.wait_for("$25,931,382.67").unwrap();
    assert!(!app.shows("No open positions") && !loading(&app));
}

#[test]
fn only_the_wallets_own_rows_can_be_closed_and_a_half_is_cut_to_the_markets_places() {
    fn desk(instruments: Option<&Instruments>) -> Desk<'_> {
        Desk {
            venue: "hyperliquid",
            instruments,
            orders: None,
        }
    }
    let instrument = |symbol: &str, size_decimals: u32| Instrument {
        symbol: symbol.to_owned(),
        size_decimals,
        max_leverage: None,
        tradable: true,
        isolated_only: false,
    };
    let listed = Instruments(vec![
        instrument("BTC", 5),
        instrument("ETH", 4),
        instrument("kPEPE", 0),
    ]);
    let held = [
        Position {
            size: 0.01,
            ..position("BTC", Some(610.0))
        },
        Position {
            size: 0.0335,
            direction: Direction::Short,
            ..position("ETH", Some(100.0))
        },
        Position {
            size: 1.0,
            ..position("kPEPE", Some(50.0))
        },
        Position {
            size: 3.0,
            ..position("XYZ", Some(10.0))
        },
    ];
    let rows = |desk: Option<Desk>| {
        session::rows_of_all(&[Part {
            mark: None,
            positions: &held[..],
            prices: None,
            desk,
        }])
    };

    // Somebody else's, watched by its address: nothing closes it.
    assert!(rows(None).iter().all(|row| row.close.is_none()));
    // The wallet's own, before its exchange has said what it trades: all
    // of it can be closed, and half is not known yet.
    assert_eq!(
        rows(Some(desk(None)))[0].close,
        Some(Close {
            venue: "hyperliquid".to_owned(),
            size: 0.01,
            half: None,
        })
    );
    // Once it has: half, cut down to the market's places; none of a
    // position too small to halve, nor of a market it does not list.
    let own = rows(Some(desk(Some(&listed))));
    let halves: Vec<(&str, Option<f64>)> = own
        .iter()
        .map(|row| (row.instrument.as_str(), row.close.as_ref().unwrap().half))
        .collect();
    assert_eq!(
        halves,
        [
            ("BTC", Some(0.005)),
            ("ETH", Some(0.0167)),
            ("kPEPE", None),
            ("XYZ", None),
        ]
    );

    // What is typed is the size closed: on the other side, at the market's
    // price, only ever reducing.
    let (btc, eth) = (&own[0], &own[1]);
    assert_eq!(
        closing_order(btc, " 0.005 "),
        Ok(OrderRequest::closing("BTC", Direction::Long, 0.005))
    );
    let all = closing_order(eth, "0.0335").unwrap();
    assert_eq!(
        (all.side, all.size, all.reduce_only),
        (Side::Buy, 0.0335, true)
    );
    assert_eq!(
        all.kind,
        OrderKind::Market {
            slippage: MARKET_SLIPPAGE
        }
    );
    // No size, and more than is held.
    for nothing in ["", "0", "abc", "1,5", "-1"] {
        assert_eq!(
            closing_order(btc, nothing),
            Err("Enter a size above zero (1.5).".to_owned()),
            "{nothing}"
        );
    }
    assert_eq!(
        closing_order(btc, "0.02"),
        Err("The position is 0.01 BTC: enter that or less.".to_owned())
    );
    // A row that is not the wallet's closes nothing.
    assert!(closing_order(&rows(None)[0], "0.01").is_err());
}

#[test]
fn a_price_heard_since_the_report_moves_the_figures_and_not_the_rows() {
    // A short entered at 60,000 and reported at 61,000, and a long reported
    // at 2,100, worth less.
    let btc = Position {
        direction: Direction::Short,
        size: 0.1,
        entry_price: Some(60_000.0),
        mark_price: Some(61_000.0),
        unrealized_pnl: Some(-100.0),
        return_on_equity: Some(-0.0833),
        leverage: Some(Leverage {
            value: 5.0,
            mode: MarginMode::Cross,
        }),
        ..position("BTC", Some(6_100.0))
    };
    let eth = Position {
        entry_price: Some(2_000.0),
        mark_price: Some(2_100.0),
        unrealized_pnl: Some(200.0),
        return_on_equity: Some(0.5),
        ..position("ETH", Some(4_200.0))
    };
    let held = [eth, btc];
    let table = |prices: Option<&HashMap<String, f64>>| {
        session::table_of(&[Part {
            mark: None,
            positions: &held[..],
            prices,
            desk: None,
        }])
    };
    let figures =
        |row: &session::Row| [&row.instrument, &row.mark, &row.value, &row.pnl].map(String::clone);

    // As reported, and what they come to.
    let (rows, totals) = table(None);
    assert_eq!(
        figures(&rows[0]),
        ["BTC", "61000.0", "$6,100.00", "-$100.00 (-8.33%)"]
    );
    assert_eq!(
        figures(&rows[1]),
        ["ETH", "2100.00", "$4,200.00", "+$200.00 (+50.00%)"]
    );
    assert_eq!(
        totals,
        Totals {
            count: 2,
            value: "$10,300.00".to_owned(),
            pnl: "+$100.00".to_owned(),
            tone: Some(Tone::Gain),
        }
    );

    // A price heard of one: that row at it, the other as reported. The
    // short has made what the fall is worth, over the margin it was opened
    // with.
    let heard = HashMap::from([("BTC".to_owned(), 59_000.0)]);
    let (rows, totals) = table(Some(&heard));
    assert_eq!(
        figures(&rows[0]),
        ["BTC", "59000.0", "$5,900.00", "+$100.00 (+8.33%)"]
    );
    assert_eq!(rows[0].tone, Some(Tone::Gain));
    assert_eq!(figures(&rows[1])[2], "$4,200.00");
    assert_eq!(
        (totals.value.as_str(), totals.pnl.as_str()),
        ("$10,100.00", "+$300.00")
    );

    // Prices of both, one of which makes its row worth less than the other:
    // the rows stay where the reports put them.
    let heard = HashMap::from([("BTC".to_owned(), 30_000.0), ("ETH".to_owned(), 2_000.0)]);
    let (rows, totals) = table(Some(&heard));
    assert_eq!(
        figures(&rows[0]),
        ["BTC", "30000.0", "$3,000.00", "+$3,000.00 (+249.90%)"]
    );
    assert_eq!(
        figures(&rows[1]),
        ["ETH", "2000.00", "$4,000.00", "$0.00 (0.00%)"]
    );
    assert_eq!(rows[1].tone, Some(Tone::Flat));
    assert_eq!(
        (totals.value.as_str(), totals.pnl.as_str(), totals.tone),
        ("$7,000.00", "+$3,000.00", Some(Tone::Gain))
    );
    // A price of something not held changes nothing.
    let other = HashMap::from([("SOL".to_owned(), 1.0)]);
    assert_eq!(table(Some(&other)), table(None));

    // Nothing held comes to nothing.
    let (rows, totals) = session::table_of(&[]);
    assert!(rows.is_empty());
    assert_eq!((totals.count, totals.tone), (0, None));
    assert_eq!([totals.value.as_str(), totals.pnl.as_str()], [NONE; 2]);
}

#[test]
fn the_orders_that_wait_to_close_a_position_are_its_exits() {
    let order = |instrument: &str, side: Side, trigger: Option<f64>, whole: bool, reduces: bool| {
        OpenOrder {
            id: "1".to_owned(),
            instrument: instrument.to_owned(),
            side,
            price: None,
            size: None,
            reduce_only: reduces,
            kind: "Stop Market".to_owned(),
            trigger_price: trigger,
            exits: Exits::default(),
            whole_position: whole,
        }
    };
    let long = Position {
        size: 0.01,
        entry_price: Some(60_000.0),
        mark_price: Some(61_000.0),
        ..position("BTC", Some(610.0))
    };
    let exits = |take_profit: Option<f64>, stop_loss: Option<f64>| Exits {
        take_profit,
        stop_loss,
    };
    let waiting = OpenOrders(vec![
        // The position's own, and one of each farther off.
        order("BTC", Side::Sell, Some(65_000.0), true, true),
        order("BTC", Side::Sell, Some(58_000.0), true, true),
        order("BTC", Side::Sell, Some(70_000.0), false, true),
        order("BTC", Side::Sell, Some(50_000.0), false, true),
        // None of these closes it: one on its own side, one that waits for
        // no price, one that would open a short, and another market's.
        order("BTC", Side::Buy, Some(62_000.0), false, true),
        order("BTC", Side::Sell, None, false, true),
        order("BTC", Side::Sell, Some(61_500.0), false, false),
        order("ETH", Side::Sell, Some(60_500.0), true, true),
    ]);
    assert_eq!(
        exits_on(&waiting, &long),
        exits(Some(65_000.0), Some(58_000.0))
    );

    // A short's are the other way round: its profit is below its price.
    let short = Position {
        direction: Direction::Short,
        ..long.clone()
    };
    let waiting_short = OpenOrders(vec![
        order("BTC", Side::Buy, Some(55_000.0), true, true),
        order("BTC", Side::Buy, Some(63_000.0), true, true),
    ]);
    assert_eq!(
        exits_on(&waiting_short, &short),
        exits(Some(55_000.0), Some(63_000.0))
    );
    // The long's are none of the short's, and one alone is one.
    assert_eq!(exits_on(&waiting_short, &long), exits(None, None));
    let stop = OpenOrders(vec![order("BTC", Side::Sell, Some(58_000.0), true, true)]);
    assert_eq!(exits_on(&stop, &long), exits(None, Some(58_000.0)));
    // No price to tell them apart by: none is said.
    let unpriced = Position {
        entry_price: None,
        mark_price: None,
        ..long.clone()
    };
    assert_eq!(exits_on(&waiting, &unpriced), exits(None, None));

    // A row says them for the wallet's own account alone: dashes until its
    // orders have arrived, and nothing for anybody else's.
    let held = [long];
    let row = |desk: Option<Desk>| {
        let part = Part {
            mark: None,
            positions: &held[..],
            prices: None,
            desk,
        };
        session::rows_of_all(&[part]).remove(0).exits
    };
    let desk = |orders| Desk {
        venue: "hyperliquid",
        instruments: None,
        orders,
    };
    assert_eq!(row(None), None);
    assert_eq!(row(Some(desk(None))), Some(format!("{NONE} / {NONE}")));
    assert_eq!(
        row(Some(desk(Some(&waiting)))).as_deref(),
        Some("65000.0 / 58000.0")
    );
    assert_eq!(
        row(Some(desk(Some(&stop)))),
        Some(format!("{NONE} / 58000.0"))
    );
}

const CLOSE: &str = "label:Close the BTC long";
const REDUCE: &str = "label:Reduce the BTC long";
const CLOSE_SIZE: &str = "id:positions-close-size";
const CLOSE_SEND: &str = "id:positions-close-send";

/// A workspace of one Positions widget watching the stand-in wallet's own
/// account, over the stubbed Hyperliquid that keeps a book.
#[component]
fn Own() -> impl IntoElement {
    trade_bench::workspace(WidgetKind::Positions, Some(account(ACCOUNT)))
}

/// Waits for the field `selector` to hold `text`.
fn holds(app: &mut Headless, selector: &str, text: &str) -> bool {
    let held = app.wait_until("the field's text", WAIT, |screen| {
        screen.input_text(selector).ok().flatten() == Some(text)
    });
    held.is_ok()
}

/// Where layout put the view whose test id is `id` (the first, of several).
fn placed(app: &Headless, id: &str) -> Bounds<Points> {
    let nodes = app.screen().layout_nodes().into_iter();
    nodes
        .filter(|node| node.test_id.as_deref() == Some(id))
        .map(|node| node.bounds)
        .next()
        .unwrap_or_else(|| panic!("{id} is not on screen"))
}

#[test]
fn the_wallets_own_positions_are_reduced_and_closed_at_market() {
    let exchange = Exchange::new();
    exchange.hold("BTC", "0.01", "60000.0", "610.0", "10.0");
    let Bench {
        mut app,
        network,
        exchange,
        ..
    } = start_on(
        Root::of::<Own>(),
        1280.0,
        &exchange,
        BTreeMap::new(),
        Clock::system(),
    );

    // Watched by its address alone the account is only read: its position
    // shows, and nothing closes it.
    app.wait_for("$610.00").unwrap();
    assert!(!app.shows("CLOSE AT MARKET"));
    assert!(!app.screen().matches("id:positions-close").unwrap());

    // The wallet connects and the account is its own: its row has the two
    // that start closing it, under a title that says how.
    app.press("the wallet connects").unwrap();
    app.wait_for(CLOSE).unwrap();
    assert!(app.screen().matches(REDUCE).unwrap());
    assert!(app.shows("CLOSE AT MARKET"));

    // Trading is not enabled for it yet: what there is to do is said, and
    // nothing opens or is sent.
    app.press(CLOSE).unwrap();
    app.wait_for(
        "Enable trading on Hyperliquid to close positions here: press your address in the top \
         bar, then Enable.",
    )
    .unwrap();
    assert!(!app.screen().matches(CLOSE_SIZE).unwrap());
    assert!(sent(&network, "exchange").is_empty());

    // Enabled: what was said goes.
    app.press("the account enables trading").unwrap();
    app.wait_for("hyperliquid is Ready").unwrap();
    app.wait_for_gone("id:positions-said", WAIT).unwrap();

    // The closing cell ends the table, as wide whatever it holds.
    let left = |cell: Bounds<Points>| cell.origin.x.0;
    let right = |cell: Bounds<Points>| cell.origin.x.0 + cell.size.width.0;
    let meet = |a: f32, b: f32| (a - b).abs() < 0.5;
    let cell = placed(&app, "positions-close");
    assert_eq!(cell.size.width, px(168.0));
    assert!(meet(right(cell), right(placed(&app, "positions-table"))));

    // Reduce starts with half the position, in the row's own cell: the
    // size, the cell that sends (it says the side, and its label the whole
    // order) and ×, edge to edge where the two were.
    app.press(REDUCE).unwrap();
    assert!(holds(&mut app, CLOSE_SIZE, "0.005"));
    assert!(!app.screen().matches(CLOSE).unwrap());
    assert!(
        app.screen()
            .matches("label:Sell 0.005 BTC at market")
            .unwrap()
    );
    assert!(
        app.screen()
            .matches(&format!("{CLOSE_SEND} >> text:Sell"))
            .unwrap()
    );
    assert_eq!(placed(&app, "positions-close"), cell);
    let [size, send, cancel] =
        ["size", "send", "cancel"].map(|id| placed(&app, &format!("positions-close-{id}")));
    assert!(left(size) >= left(cell) && left(size) <= left(cell) + 1.5);
    assert!(meet(right(size), left(send)) && meet(right(send), left(cancel)));
    assert!(meet(right(cancel), right(cell)));
    assert_eq!(size.size.height, px(36.0));
    assert_eq!((send.size.width, cancel.size.width), (px(48.0), px(28.0)));

    // A size finer than the market's is refused here, in words; one worth
    // too little by the exchange, in its own; more than is held, and
    // nothing, before anything is asked. The cell stays for another try.
    let tries = |app: &mut Headless, typed: &str, said: &str| {
        app.clear(CLOSE_SIZE).unwrap();
        app.type_text(CLOSE_SIZE, typed).unwrap();
        app.press(CLOSE_SEND).unwrap();
        app.wait_for(said).unwrap();
        assert!(app.screen().matches(CLOSE_SIZE).unwrap());
    };
    tries(
        &mut app,
        "0.000001",
        "A size of BTC has 5 decimal places at most.",
    );
    assert!(acted(&network).is_empty());
    tries(&mut app, "0.0001", "Order must have minimum value of $10.");
    assert_eq!(acted(&network).len(), 1);
    tries(
        &mut app,
        "0.02",
        "The position is 0.01 BTC: enter that or less.",
    );
    tries(&mut app, "", "Enter a size above zero (1.5).");
    assert_eq!(acted(&network).len(), 1);

    // × lets it go with nothing sent; started again, it is filled in again.
    app.press("id:positions-close-cancel").unwrap();
    app.wait_for(CLOSE).unwrap();
    assert!(!app.screen().matches(CLOSE_SIZE).unwrap());
    app.press(REDUCE).unwrap();
    assert!(holds(&mut app, CLOSE_SIZE, "0.005"));
    assert!(!app.shows("Enter a size above zero (1.5)."));

    // Sent: one order that sells the size, cancels what does not fill at
    // once, 5% under the mid in five figures, and only reduces.
    app.press(CLOSE_SEND).unwrap();
    app.wait_for("Sold 0.005 BTC at 61000.0.").unwrap();
    assert_eq!(
        acted(&network)[1],
        json!({
            "type": "order",
            "orders": [{
                "a": 0, "b": false, "p": "57962", "s": "0.005", "r": true,
                "t": { "limit": { "tif": "Ioc" } },
            }],
            "grouping": "na",
        })
    );
    // The cell is the row's two again, and the row is what is left.
    app.wait_for(CLOSE).unwrap();
    assert!(!app.screen().matches(CLOSE_SIZE).unwrap());
    app.wait_for("id:positions-table >> text:0.005").unwrap();
    app.wait_for("$305.00").unwrap();

    // What was said is taken away with ×.
    app.press("id:positions-said-dismiss").unwrap();
    app.wait_for_gone("id:positions-said", WAIT).unwrap();

    // Close starts with all of it, and Enter sends as the cell does. With
    // nothing left to close, the column goes.
    app.press(CLOSE).unwrap();
    assert!(holds(&mut app, CLOSE_SIZE, "0.005"));
    app.type_text(CLOSE_SIZE, "\n").unwrap();
    app.wait_for("Sold 0.005 BTC at 61000.0.").unwrap();
    app.wait_for("No open positions").unwrap();
    assert_eq!(acted(&network)[2]["orders"][0]["s"], "0.005");
    assert!(!app.shows("CLOSE AT MARKET"));

    // A short the account comes to hold, as its stream says it: bought
    // back, 5% over the mid.
    exchange.hold("ETH", "-0.5", "1900.0", "945.7", "4.3");
    app.wait_for("label:Close the ETH short").unwrap();
    app.press("label:Close the ETH short").unwrap();
    assert!(holds(&mut app, CLOSE_SIZE, "0.5"));
    assert!(app.screen().matches("label:Buy 0.5 ETH at market").unwrap());
    app.press(CLOSE_SEND).unwrap();
    app.wait_for("Bought 0.5 ETH at 61000.0.").unwrap();
    let bought = &acted(&network)[3]["orders"][0];
    assert_eq!(
        bought,
        &json!({
            "a": 1, "b": true, "p": "1986", "s": "0.5", "r": true,
            "t": { "limit": { "tif": "Ioc" } },
        })
    );
    app.wait_for("No open positions").unwrap();

    // Another account in the wallet: what is watched is somebody else's
    // again. The position being closed is let go, and nothing closes it.
    exchange.hold("BTC", "0.01", "60000.0", "610.0", "10.0");
    app.wait_for(CLOSE).unwrap();
    app.press(CLOSE).unwrap();
    assert!(holds(&mut app, CLOSE_SIZE, "0.01"));
    app.press("the wallet changes account").unwrap();
    app.wait_for_gone(CLOSE_SIZE, WAIT).unwrap();
    assert!(!app.screen().matches("id:positions-close").unwrap());
    assert!(app.shows("$610.00"));
    assert_eq!(acted(&network).len(), 4);
}

/// Waits for `done`, which is about something that shows nowhere on screen
/// (what the app sent its exchange): the app runs on, on its own thread,
/// while the test looks again.
fn eventually(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Whether the view `id` shows `text`.
fn says(app: &Headless, id: &str, text: &str) -> bool {
    let selector = format!("id:{id} >> text:{text}");
    app.screen().matches(&selector).unwrap()
}

/// Whether a column of the table is titled `title` (as drawn: capitals).
fn titled(app: &Headless, title: &str) -> bool {
    app.screen().texts().contains(&title)
}

/// What the app subscribes to follow `feed` (`activeAssetCtx`, `bbo`) of
/// `coin`.
fn asset(feed: &str, coin: &str) -> serde_json::Value {
    json!({ "type": feed, "coin": coin })
}

#[test]
fn prices_heard_between_reports_move_the_rows_and_what_they_come_to() {
    let exchange = Exchange::new();
    exchange.hold("BTC", "0.01", "60000.0", "610.0", "10.0");
    let Bench {
        mut app,
        network,
        exchange,
        ..
    } = start_on(
        Root::of::<Own>(),
        1280.0,
        &exchange,
        BTreeMap::new(),
        Clock::system(),
    );

    // As its report says, and what the one position comes to.
    app.wait_for("$610.00").unwrap();
    assert!(says(&app, "positions-table", "61000.0"));
    assert!(says(&app, "positions-table", "+$10.00 (+8.33%)"));
    for (total, figure) in [("count", "1"), ("value", "$610.00"), ("pnl", "+$10.00")] {
        assert!(
            says(&app, &format!("positions-total-{total}"), figure),
            "{total}"
        );
    }
    for title in ["POSITIONS", "TOTAL VALUE", "TOTAL PNL"] {
        assert!(says(&app, "positions-totals", title), "{title}");
    }
    // Valued at the live price while the settings say nothing else.
    assert!(titled(&app, "PRICE") && !titled(&app, "MARK"));

    // The price of what it holds is followed, with no wallet: the market's
    // context, then, once that has spoken, its best bid and offer; none of
    // the account's topics.
    let (context, book) = (asset("activeAssetCtx", "BTC"), asset("bbo", "BTC"));
    eventually("the price being followed", || {
        exchange.subscribed().contains(&book)
    });
    assert_eq!(exchange.subscribed(), [context, book]);
    let reports = count(&asked(&network), "clearinghouseState", ACCOUNT);

    // The best bid and offer rise: the price is the middle of them, and
    // with it move the worth, the profit with its return (on the margin
    // the report measures it against) and the totals.
    exchange.quote("BTC", "61999.5", "62000.5");
    app.wait_for("id:positions-table >> text:62000.0").unwrap();
    assert!(says(&app, "positions-table", "$620.00"));
    assert!(says(&app, "positions-table", "+$20.00 (+16.66%)"));
    assert!(says(&app, "positions-total-value", "$620.00"));
    assert!(says(&app, "positions-total-pnl", "+$20.00"));
    assert!(!app.shows("$610.00"));
    // They fall under the entry: a loss.
    exchange.quote("BTC", "58999.5", "59000.5");
    app.wait_for("id:positions-table >> text:59000.0").unwrap();
    assert!(says(&app, "positions-table", "$590.00"));
    assert!(says(&app, "positions-table", "-$10.00 (-8.33%)"));
    assert!(says(&app, "positions-total-pnl", "-$10.00"));
    // Nothing was asked for any of it.
    assert_eq!(
        count(&asked(&network), "clearinghouseState", ACCOUNT),
        reports
    );

    // The stream stops (the connection is lost, and what comes back says
    // no price): the row is its report again, not the last price heard.
    exchange.hush(true);
    exchange.socket.sever();
    app.wait_for("id:positions-table >> text:$610.00").unwrap();
    assert!(says(&app, "positions-table", "61000.0"));
    assert!(says(&app, "positions-table", "+$10.00 (+8.33%)"));
    assert!(says(&app, "positions-total-pnl", "+$10.00"));
    // It speaks again: the price it says now.
    eventually("the socket coming back", || {
        exchange.socket.connections() == 2 && exchange.subscribed().len() == 4
    });
    exchange.hush(false);
    exchange.quote("BTC", "62999.5", "63000.5");
    app.wait_for("id:positions-table >> text:$630.00").unwrap();
    assert!(says(&app, "positions-total-value", "$630.00"));
}

#[test]
fn the_settings_say_which_price_values_the_rows() {
    let exchange = Exchange::new();
    exchange.hold("BTC", "0.01", "60000.0", "610.0", "10.0");
    let Bench {
        mut app, exchange, ..
    } = start_on(
        Root::of::<Own>(),
        1280.0,
        &exchange,
        BTreeMap::new(),
        Clock::system(),
    );
    app.wait_for("$610.00").unwrap();
    let book = asset("bbo", "BTC");
    let books = || {
        let subscribed = exchange.subscribed();
        subscribed.iter().filter(|topic| **topic == book).count()
    };
    eventually("the live price being followed", || books() == 1);
    assert!(titled(&app, "PRICE"));

    // The exchange's mark, picked in the settings: the column says so, and
    // the best bid and offer are let go.
    const VALUED_AT: &str = "id:positions-settings >> label:Valued at";
    app.press("id:positions-account").unwrap();
    app.wait_for("id:positions-settings").unwrap();
    app.press(VALUED_AT).unwrap();
    app.press("role:option[Mark price]").unwrap();
    app.wait_until("the column being the mark's", WAIT, |screen| {
        screen.texts().contains(&"MARK")
    })
    .unwrap();
    assert!(!titled(&app, "PRICE"));
    eventually("the best bid and offer being let go", || {
        let let_go = |frame: &String| frame.contains("unsubscribe") && frame.contains("bbo");
        exchange.socket.sent().iter().any(let_go)
    });
    // The rows follow the mark, as the exchange says it.
    exchange.mark("BTC", "62000.0");
    app.wait_for("id:positions-table >> text:62000.0").unwrap();
    assert!(says(&app, "positions-table", "$620.00"));
    assert!(says(&app, "positions-total-pnl", "+$20.00"));

    // And the live price again: the best bid and offer are followed anew,
    // and the rows are at the middle of them, not at the mark.
    app.press(VALUED_AT).unwrap();
    app.press("role:option[Live price]").unwrap();
    app.wait_until("the column being the live price's", WAIT, |screen| {
        screen.texts().contains(&"PRICE")
    })
    .unwrap();
    eventually("the live price being followed again", || books() == 2);
    app.wait_for("id:positions-table >> text:$610.00").unwrap();
    assert!(says(&app, "positions-table", "61000.0"));
}

#[test]
fn a_market_hyperliquid_does_not_know_is_never_asked_for_its_book() {
    // Hyperliquid closes the connection over the best bid and offer of a
    // market it does not know, and with it every account followed.
    let exchange = Exchange::new();
    exchange.hold("NOPE", "1.0", "1.0", "1.0", "0.0");
    let Bench {
        mut app, exchange, ..
    } = start_on(
        Root::of::<Own>(),
        1280.0,
        &exchange,
        BTreeMap::new(),
        Clock::system(),
    );
    // Its context is asked for, which Hyperliquid refuses: the row stays as
    // its report says.
    app.wait_for("id:positions-table >> text:NOPE").unwrap();
    eventually("its context being asked for", || {
        exchange
            .subscribed()
            .contains(&asset("activeAssetCtx", "NOPE"))
    });
    assert!(says(&app, "positions-table", "$1.00"));

    // The account comes to hold a market Hyperliquid knows, whose book is
    // asked for once its context has spoken. By then the other's would have
    // been asked for too, and it never was.
    app.press("the wallet connects").unwrap();
    let account = json!({ "type": "clearinghouseState", "user": ACCOUNT });
    eventually("the account being followed", || {
        exchange.subscribed().contains(&account)
    });
    exchange.hold("BTC", "0.01", "60000.0", "610.0", "10.0");
    app.wait_for("$610.00").unwrap();
    eventually("the known market's book being followed", || {
        exchange.subscribed().contains(&asset("bbo", "BTC"))
    });
    let books: Vec<serde_json::Value> = exchange
        .subscribed()
        .into_iter()
        .filter(|topic| topic["type"] == "bbo")
        .collect();
    assert_eq!(books, [asset("bbo", "BTC")]);
    assert_eq!(exchange.socket.connections(), 1);
}

#[test]
fn the_wallets_own_account_is_followed_live_and_says_its_exits() {
    let exchange = Exchange::new();
    exchange.hold("BTC", "0.01", "60000.0", "610.0", "10.0");
    let Bench {
        mut app,
        network,
        exchange,
        ..
    } = start_on(
        Root::of::<Own>(),
        1280.0,
        &exchange,
        BTreeMap::new(),
        Clock::system(),
    );
    app.wait_for("$610.00").unwrap();
    // Somebody else's, as far as the app knows: no exits are read of it.
    assert!(!app.shows("TP / SL"));

    // The wallet connects: the account is followed from then on, trading
    // enabled or not, and its row says what waits on it.
    app.press("the wallet connects").unwrap();
    app.wait_for("TP / SL").unwrap();
    let account = json!({ "type": "clearinghouseState", "user": ACCOUNT });
    eventually("the account being followed", || {
        exchange.subscribed().contains(&account)
    });
    assert!(app.shows("hyperliquid is Locked"));
    app.wait_for(&format!("{NONE} / {NONE}")).unwrap();

    // A take profit and a stop loss placed elsewhere show as they come to
    // wait, and go as they are taken away.
    exchange.exit(7001, "BTC", "A", "65000.0", true);
    app.wait_for(&format!("65000.0 / {NONE}")).unwrap();
    exchange.exit(7002, "BTC", "A", "58000.0", false);
    app.wait_for("65000.0 / 58000.0").unwrap();
    exchange.withdraw(7001);
    app.wait_for(&format!("{NONE} / 58000.0")).unwrap();

    // What the market does to the account shows as its stream says it,
    // at the price last heard, and with no request made for it.
    let reports = count(&asked(&network), "clearinghouseState", ACCOUNT);
    exchange.hold("BTC", "0.02", "60000.0", "1220.0", "20.0");
    app.wait_for("$1,220.00").unwrap();
    assert!(says(&app, "positions-total-value", "$1,220.00"));
    exchange.quote("BTC", "61999.5", "62000.5");
    app.wait_for("$1,240.00").unwrap();
    assert!(says(&app, "positions-table", "+$40.00 (+16.66%)"));
    assert_eq!(
        count(&asked(&network), "clearinghouseState", ACCOUNT),
        reports
    );
    // Nothing was signed or sent for any of it.
    assert!(sent(&network, "exchange").is_empty());
}

/// A workspace of one Positions widget watching the stand-in wallet's
/// account on Paper, an exchange that is none and has nothing a venue may
/// leave out.
#[component]
fn OnPaper() -> impl IntoElement {
    let saved = json!({ "account": on("paper", ACCOUNT) });
    trade_bench::workspace_with_paper(WidgetKind::Positions, Some(saved))
}

#[test]
fn another_exchanges_positions_are_reduced_and_closed_through_the_same_table() {
    let Bench {
        mut app,
        network,
        exchange,
        ..
    } = start_on(
        Root::of::<OnPaper>(),
        1280.0,
        &Exchange::new(),
        BTreeMap::new(),
        Clock::system(),
    );
    const CLOSE: &str = "label:Close the BTC-PERP long";
    const REDUCE: &str = "label:Reduce the BTC-PERP long";

    // Its position, as its report says: nothing prices this exchange, so
    // the row is the report.
    app.wait_for("$220.00").unwrap();
    assert!(says(&app, "positions-table", "BTC-PERP"));
    assert!(says(&app, "positions-table", "110.000"));
    assert!(says(&app, "positions-table", "+$20.00"));
    assert!(
        app.screen()
            .matches("label:Change account: Paper 0x1479…9325")
            .unwrap()
    );
    assert!(!app.screen().matches("id:positions-close").unwrap());

    // The wallet's own: it can be closed, and its exits are said (none: the
    // exchange lists no orders that wait).
    app.press("the wallet connects").unwrap();
    app.wait_for(CLOSE).unwrap();
    app.wait_for(&format!("{NONE} / {NONE}")).unwrap();
    // Trading is enabled exchange by exchange: what there is to do names
    // this one.
    app.press(REDUCE).unwrap();
    app.wait_for(
        "Enable trading on Paper to close positions here: press your address in the top bar, \
         then Enable.",
    )
    .unwrap();
    app.press("the account enables trading on Paper").unwrap();
    app.wait_for("paper is Ready").unwrap();
    assert!(app.shows("hyperliquid is Locked"));

    // Half, to this exchange's size places (one), sold at its mark.
    app.press(REDUCE).unwrap();
    assert!(holds(&mut app, CLOSE_SIZE, "1"));
    assert!(
        app.screen()
            .matches("label:Sell 1 BTC-PERP at market")
            .unwrap()
    );
    app.press(CLOSE_SEND).unwrap();
    app.wait_for("Sold 1 BTC-PERP at 110.000.").unwrap();
    app.wait_for("$110.00").unwrap();
    // What it refuses is said in its words, and the cell stays.
    app.press(CLOSE).unwrap();
    app.clear(CLOSE_SIZE).unwrap();
    app.type_text(CLOSE_SIZE, "0.25").unwrap();
    app.press(CLOSE_SEND).unwrap();
    app.wait_for("A size of BTC-PERP has 1 decimal place at most.")
        .unwrap();
    // The rest.
    app.clear(CLOSE_SIZE).unwrap();
    app.type_text(CLOSE_SIZE, "1\n").unwrap();
    app.wait_for("No open positions").unwrap();

    // None of it asked Hyperliquid anything, over either of its ways.
    assert!(asked(&network).is_empty());
    assert!(sent(&network, "exchange").is_empty());
    assert!(exchange.socket.sent().is_empty());
}
