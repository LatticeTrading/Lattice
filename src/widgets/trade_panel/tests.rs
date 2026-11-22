//! The Trade Panel over a stand-in wallet and a stubbed Hyperliquid that
//! keeps a book of what was placed (the bench every trading widget's tests
//! share, [`trade_bench`]): its settings, what it makes of what is typed,
//! and, headless, the way from no wallet through enabling to orders placed,
//! refused and cancelled.

use std::{
    collections::BTreeMap,
    time::{Duration, SystemTime},
};

use nsh_core::{element::Root, prelude::*, time::Clock};
use nsh_testing::{Headless, StubNetwork, WAIT};
use serde_json::{Value, json};

use super::{
    session::{
        Draft, Listing, SLIPPAGE, exits_of, leverage_of, leverage_text, open_order_line,
        order_amount, order_line, order_price, placed_line, position_in, protected_line,
        request_of, typed,
    },
    settings::TradePanelSettings,
};
use crate::{
    data::{
        ferris::MarketSelection,
        portfolio::{Direction, Leverage, MarginAccount, MarginMode, Position},
        trading::{
            Exits, Instrument, Instruments, OpenOrder, OrderKind, OrderRequest, Placed, Side,
            TimeInForce,
        },
    },
    link::LinkColor,
    widgets::{
        WidgetKind,
        common::{
            accounts::NONE,
            trade_bench::{
                self, ACCOUNT, ACCOUNT_SHORT, Bench, Exchange, KEPT, acted, sent, start, start_in,
                start_live, start_on,
            },
        },
    },
};

const ENABLE: &str = "label:Enable trading on Hyperliquid";
const SEND: &str = "id:trade-panel-send";
const SIZE: &str = "id:trade-panel-size";
const PRICE: &str = "id:trade-panel-price";
const MARKET: &str = "id:trade-panel-market";

#[test]
fn settings_save_what_differs_and_read_leniently() {
    // Defaults save nothing: the first exchange, BTC, the blue link,
    // orders confirmed.
    let defaults = TradePanelSettings::default();
    assert_eq!(serde_json::to_value(&defaults).unwrap(), json!({}));
    assert_eq!(defaults.venue(), None);
    assert_eq!(defaults.instrument(), "BTC");
    assert_eq!(defaults.link(), Some(LinkColor::DEFAULT));
    assert!(defaults.confirm());

    let settings = defaults
        .clone()
        .with_venue(Some("hyperliquid"))
        .with_instrument(" kPEPE ")
        .with_link(None)
        .with_confirm(false);
    let saved = json!({
        "venue": "hyperliquid",
        "instrument": "kPEPE",
        "linkColor": "",
        "confirmOrders": false,
    });
    assert_eq!(serde_json::to_value(&settings).unwrap(), saved);
    assert_eq!(TradePanelSettings::from(saved), settings);
    // Back to the defaults is back to nothing saved.
    assert_eq!(
        settings
            .with_venue(None)
            .with_instrument("BTC")
            .with_link(Some(LinkColor::DEFAULT))
            .with_confirm(true),
        defaults
    );

    // Anything else reads as the defaults.
    for damaged in [
        json!(null),
        json!([]),
        json!({ "venue": 7, "instrument": ["BTC"], "confirmOrders": "no" }),
        json!({ "instrument": "  " }),
    ] {
        assert_eq!(
            TradePanelSettings::from(damaged.clone()),
            defaults,
            "{damaged}"
        );
    }
    // A link color it does not know is the default one, as the Chart's.
    assert_eq!(
        TradePanelSettings::from(json!({ "linkColor": "mauve" })).link(),
        Some(LinkColor::DEFAULT)
    );

    // A market sent to its link color is traded by its asset, whichever
    // exchange the market is on.
    let linked = defaults.clone().with_linked_market(MarketSelection {
        exchange: "binance".to_owned(),
        symbol: "ETH/USDT".to_owned(),
        base: "ETH".to_owned(),
        ..MarketSelection::bybit_btc_usdt()
    });
    assert_eq!(linked.instrument(), "ETH");
    assert_eq!(
        linked.with_linked_market(MarketSelection::bybit_btc_usdt()),
        defaults
    );
}

fn draft(price: &str, size: &str) -> Draft {
    Draft {
        side: Side::Buy,
        market: false,
        time_in_force: TimeInForce::GoodTilCancelled,
        price: price.to_owned(),
        size: size.to_owned(),
        reduce_only: false,
        take_profit: String::new(),
        stop_loss: String::new(),
    }
}

#[test]
fn what_is_typed_is_an_order_or_what_is_missing_of_it() {
    // A plain decimal is a number; nothing else is read as one.
    assert_eq!(typed(" 0.01 "), Some(0.01));
    assert_eq!(typed("61000"), Some(61_000.0));
    assert_eq!(typed(".5"), Some(0.5));
    for no_number in [
        "", " ", "1,5", "1 000", "-1", "1e3", "inf", "NaN", "0x10", "1.2.3",
    ] {
        assert_eq!(typed(no_number), None, "{no_number}");
    }

    let order = request_of(&draft("61000.5", "0.01"), "BTC", None).unwrap();
    assert_eq!(
        order,
        OrderRequest {
            instrument: "BTC".to_owned(),
            side: Side::Buy,
            size: 0.01,
            kind: OrderKind::Limit {
                price: 61_000.5,
                time_in_force: TimeInForce::GoodTilCancelled,
            },
            reduce_only: false,
            exits: Exits::default(),
        }
    );
    assert_eq!(order_line(&order), "Buy 0.01 BTC at 61000.5");

    // A market order needs no price, and says so.
    let market = Draft {
        side: Side::Sell,
        market: true,
        reduce_only: true,
        ..draft("not a price", "2")
    };
    let order = request_of(&market, "ETH", None).unwrap();
    assert_eq!(order.kind, OrderKind::Market { slippage: SLIPPAGE });
    assert!(order.reduce_only);
    assert_eq!(order_line(&order), "Sell 2 ETH at market");

    // What is missing is said, the size first.
    assert_eq!(
        request_of(&draft("61000", ""), "BTC", None),
        Err("Enter a size above zero (1.5).")
    );
    assert_eq!(
        request_of(&draft("61000", "0"), "BTC", None),
        Err("Enter a size above zero (1.5).")
    );
    assert_eq!(
        request_of(&draft("1,5", "0.01"), "BTC", None),
        Err("Enter a price above zero (61000.5).")
    );
}

#[test]
fn what_became_of_an_order_is_said_in_words() {
    let order = request_of(&draft("61000", "0.01"), "BTC", None).unwrap();
    let resting = Placed::Resting {
        id: "1001".to_owned(),
    };
    assert_eq!(
        placed_line(&order, &resting),
        "Resting: Buy 0.01 BTC at 61000 (order 1001)."
    );
    let filled = Placed::Filled {
        id: "1002".to_owned(),
        size: 0.01,
        price: 60_990.0,
    };
    assert_eq!(placed_line(&order, &filled), "Bought 0.01 BTC at 60990.0.");
    assert_eq!(
        placed_line(&order, &Placed::Accepted),
        "Accepted: Buy 0.01 BTC at 61000."
    );

    let open = OpenOrder {
        id: "91490942".to_owned(),
        instrument: "BTC".to_owned(),
        side: Side::Sell,
        price: Some(29_792.0),
        size: Some(4.5),
        reduce_only: false,
        kind: "Limit".to_owned(),
        trigger_price: None,
        exits: Exits::default(),
        whole_position: false,
    };
    assert_eq!(open_order_line(&open), "Sell 4.5 BTC at 29792.0");

    // One that waits is known by the price it waits for (what it may trade
    // at past it is only a bound), and the exit of a whole position has no
    // size of its own.
    let stop = OpenOrder {
        price: Some(52_200.0),
        size: Some(0.0),
        reduce_only: true,
        kind: "Stop Market".to_owned(),
        trigger_price: Some(58_000.0),
        whole_position: true,
        ..open.clone()
    };
    assert_eq!(order_price(&stop), Some(58_000.0));
    assert_eq!(order_amount(&stop), "BTC position");
    assert_eq!(open_order_line(&stop), "Sell BTC position at 58000.0");
    assert_eq!(order_price(&open), Some(29_792.0));
    assert_eq!(order_amount(&open), "4.5 BTC");
}

#[test]
fn exits_are_placed_with_an_order_each_on_its_own_side_of_its_price() {
    let with = |take_profit: &str, stop_loss: &str| Draft {
        take_profit: take_profit.to_owned(),
        stop_loss: stop_loss.to_owned(),
        ..draft("61000", "0.01")
    };
    let both = Exits {
        take_profit: Some(65_000.0),
        stop_loss: Some(58_000.0),
    };
    let order = request_of(&with("65000", " 58000 "), "BTC", None).unwrap();
    assert_eq!(order.exits, both);
    assert_eq!(
        order_line(&order),
        "Buy 0.01 BTC at 61000, TP 65000, SL 58000"
    );
    // One alone, and none.
    let stopped = request_of(&with("", "58000"), "BTC", None).unwrap();
    assert_eq!(order_line(&stopped), "Buy 0.01 BTC at 61000, SL 58000");
    assert_eq!(exits_of("", " "), Ok(Exits::default()));

    // On the wrong side of the order's price it would trigger at once.
    assert_eq!(
        request_of(&with("60000", ""), "BTC", None),
        Err("A long's take profit is above its price.")
    );
    assert_eq!(
        request_of(&with("", "61000"), "BTC", None),
        Err("A long's stop loss is below its price.")
    );
    let sell = |take_profit: &str, stop_loss: &str| Draft {
        side: Side::Sell,
        ..with(take_profit, stop_loss)
    };
    assert!(request_of(&sell("58000", "65000"), "BTC", None).is_ok());
    assert_eq!(
        request_of(&sell("65000", ""), "BTC", None),
        Err("A short's take profit is below its price.")
    );
    // What is no price is said, not sent as none.
    assert_eq!(
        request_of(&with("65k", ""), "BTC", None),
        Err("Enter a take profit price above zero, or leave it empty.")
    );
    assert_eq!(
        request_of(&with("", "0"), "BTC", None),
        Err("Enter a stop loss price above zero, or leave it empty.")
    );

    // A market order's are measured from the market's price where it is
    // known, and left to the exchange where it is not.
    let market = Draft {
        market: true,
        ..with("60000", "")
    };
    assert_eq!(
        request_of(&market, "BTC", Some(61_000.0)),
        Err("A long's take profit is above its price.")
    );
    assert!(request_of(&market, "BTC", Some(59_000.0)).is_ok());
    assert!(request_of(&market, "BTC", None).is_ok());

    // An order that only reduces opens nothing: what was typed for exits
    // is left out of it, whatever it was.
    let reducing = Draft {
        reduce_only: true,
        ..with("60000", "x")
    };
    let order = request_of(&reducing, "BTC", None).unwrap();
    assert!(order.exits.is_empty());
    assert_eq!(order_line(&order), "Buy 0.01 BTC at 61000");
}

#[test]
fn a_leverage_and_a_positions_exits_are_said_as_they_are_set() {
    let isolated = Leverage {
        value: 20.0,
        mode: MarginMode::Isolated,
    };
    assert_eq!(leverage_of(" 20 ", MarginMode::Isolated), Ok(isolated));
    assert_eq!(leverage_of("20x", MarginMode::Isolated), Ok(isolated));
    assert_eq!(leverage_text(isolated), "20x Isolated");
    for no_leverage in ["", "0", "2.5", "-3", "twenty", "x"] {
        assert_eq!(
            leverage_of(no_leverage, MarginMode::Cross),
            Err("Enter leverage as a whole number (20)."),
            "{no_leverage}"
        );
    }

    let both = Exits {
        take_profit: Some(65_000.0),
        stop_loss: Some(58_000.0),
    };
    assert_eq!(
        protected_line("BTC", both),
        "Take profit at 65000 and stop loss at 58000 placed for the BTC position."
    );
    let stop = Exits {
        take_profit: None,
        ..both
    };
    assert_eq!(
        protected_line("BTC", stop),
        "Stop loss at 58000 placed for the BTC position."
    );

    // The position held in a market is the one named exactly as it.
    let held = |instrument: &str| Position {
        instrument: instrument.to_owned(),
        direction: Direction::Long,
        size: 0.01,
        entry_price: Some(60_000.0),
        mark_price: Some(61_000.0),
        value: Some(610.0),
        unrealized_pnl: Some(10.0),
        return_on_equity: None,
        liquidation_price: None,
        margin_used: None,
        leverage: None,
    };
    let account = MarginAccount {
        positions: vec![held("ETH"), held("BTC")],
        ..MarginAccount::default()
    };
    assert_eq!(position_in(&account, "BTC"), Some(held("BTC")));
    assert_eq!(position_in(&account, "kPEPE"), None);
}

#[test]
fn an_instrument_stands_as_the_exchange_lists_it() {
    let instrument = |symbol: &str, tradable: bool| Instrument {
        symbol: symbol.to_owned(),
        size_decimals: 2,
        max_leverage: None,
        tradable,
        isolated_only: false,
    };
    let listed = Instruments(vec![instrument("BTC", true), instrument("MATIC", false)]);
    assert_eq!(Listing::of(None, "BTC"), Listing::Unknown);
    assert_eq!(
        Listing::of(Some(&listed), "btc"),
        Listing::Listed(instrument("BTC", true))
    );
    assert_eq!(
        Listing::of(Some(&listed), "MATIC"),
        Listing::Delisted("MATIC".to_owned())
    );
    assert_eq!(Listing::of(Some(&listed), "DOGE"), Listing::Missing);
}

/// A workspace of one Trade Panel with `saved` as its settings (none: its
/// defaults), over the bench's services.
fn workspace(saved: Option<Value>) -> impl IntoElement {
    trade_bench::workspace(WidgetKind::TradePanel, saved)
}

/// A Trade Panel as it is first added.
#[component]
fn Fresh() -> impl IntoElement {
    workspace(None)
}

/// One trading ETH (typed in lower case) that sends at the first press.
#[component]
fn Quick() -> impl IntoElement {
    workspace(Some(json!({ "instrument": "eth", "confirmOrders": false })))
}

/// Connects the stand-in wallet and enables Hyperliquid for its account,
/// from the panel itself.
fn enable(app: &mut Headless) {
    app.wait_for("id:trade-panel-ask").unwrap();
    app.press("Connect wallet").unwrap();
    app.wait_for("Trading off").unwrap();
    app.press(ENABLE).unwrap();
    app.wait_for("id:trade-panel-account").unwrap();
}

/// Waits for the cell that sends the order to say `says`.
fn sends(app: &mut Headless, says: &str) -> bool {
    app.wait_for(&format!("{SEND} >> text:{says}")).is_ok()
}

#[test]
fn the_panel_connects_enables_and_trades() {
    let Bench {
        mut app,
        network,
        exchange,
        ..
    } = start_live(Root::of::<Fresh>());
    // The market's best bid and offer, whose middle is between two ticks.
    exchange.quote("BTC", "61012.2", "61013.2");

    // No wallet: the panel offers to connect one, shows no ticket, and
    // asks Hyperliquid nothing.
    app.wait_for("id:trade-panel-ask").unwrap();
    assert!(app.shows("Connect wallet"));
    assert!(!app.screen().matches(SEND).unwrap());
    assert!(sent(&network, "info").is_empty());

    // Connected: the exchange is still to be enabled for the account,
    // what that asks is said, and nothing of the account is asked for yet.
    app.press("Connect wallet").unwrap();
    app.wait_for("Trading off").unwrap();
    assert!(app.screen().texts().iter().any(|text| {
        text.starts_with("Your wallet signs one approval.")
            && text.ends_with("The key cannot withdraw or transfer.")
    }));
    assert!(!app.screen().matches(SEND).unwrap());
    assert!(sent(&network, "info").is_empty());
    assert!(sent(&network, "exchange").is_empty());

    // Enabled: the piece gives way to the account's bar and the ticket.
    app.press(ENABLE).unwrap();
    app.wait_for("id:trade-panel-account").unwrap();
    assert!(!app.screen().matches("id:trade-panel-ask").unwrap());
    assert_eq!(sent(&network, "exchange").len(), 1);
    for shown in [
        "Hyperliquid",
        ACCOUNT_SHORT,
        "EQUITY",
        "AVAILABLE",
        "MARGIN RATIO",
    ] {
        assert!(app.shows(shown), "{shown}");
    }
    app.wait_for("$1,234.50").unwrap();
    // Available, and the maintenance margin over the cross equity.
    assert!(app.shows("$1,159.50") && app.shows("0.6%"));
    // The default market, with its price once it is heard: the middle of
    // its best bid and offer, followed live.
    assert_eq!(app.screen().input_text(MARKET).unwrap(), Some("BTC"));
    app.wait_for("id:trade-panel-mid >> text:61012.7").unwrap();
    app.wait_for("No open orders").unwrap();
    assert!(
        exchange
            .subscribed()
            .contains(&json!({ "type": "bbo", "coin": "BTC" }))
    );
    // What is asked for: the account, its open orders and the universe. No
    // price is: prices are heard.
    let asked = sent(&network, "info");
    for (kind, user) in [
        ("clearinghouseState", Some(ACCOUNT)),
        ("frontendOpenOrders", Some(ACCOUNT)),
        ("meta", None),
    ] {
        let found = asked
            .iter()
            .any(|body| body["type"] == kind && user.is_none_or(|user| body["user"] == user));
        assert!(found, "{kind}");
    }
    assert!(asked.iter().all(|body| body["type"] != "allMids"));
    // It moves as the market does.
    exchange.quote("BTC", "61020.2", "61021.2");
    app.wait_for("id:trade-panel-mid >> text:61020.7").unwrap();
    exchange.quote("BTC", "61012.2", "61013.2");
    app.wait_for("id:trade-panel-mid >> text:61012.7").unwrap();

    // The cell that sends says what it would send, as the order is
    // written. The mid is between two ticks: the cell that fills it in
    // writes the nearest price an order may have.
    assert!(sends(&mut app, "Buy BTC"));
    app.type_text(SIZE, "0.01").unwrap();
    app.press("id:trade-panel-use-mid").unwrap();
    assert_eq!(app.screen().input_text(PRICE).unwrap(), Some("61013"));
    assert!(sends(&mut app, "Buy 0.01 BTC at 61013"));

    // The first press asks again in place, and sends nothing; × lets the
    // order go, and so does changing it.
    app.press(SEND).unwrap();
    assert!(sends(&mut app, "Confirm: Buy 0.01 BTC at 61013"));
    assert!(acted(&network).is_empty());
    app.press("id:trade-panel-disarm").unwrap();
    assert!(sends(&mut app, "Buy 0.01 BTC at 61013"));
    app.press(SEND).unwrap();
    app.press("id:trade-panel-side-sell").unwrap();
    assert!(sends(&mut app, "Sell 0.01 BTC at 61013"));
    app.press("id:trade-panel-side-buy").unwrap();
    assert!(acted(&network).is_empty());

    // The second press sends it: BTC is the first asset, the figures as
    // Hyperliquid reads them.
    app.press(SEND).unwrap();
    app.press(SEND).unwrap();
    app.wait_for("Resting: Buy 0.01 BTC at 61013 (order 1001).")
        .unwrap();
    assert_eq!(
        acted(&network),
        [json!({
            "type": "order",
            "orders": [{
                "a": 0, "b": true, "p": "61013", "s": "0.01", "r": false,
                "t": { "limit": { "tif": "Gtc" } },
            }],
            "grouping": "na",
        })]
    );
    // Its signature is the agent's, on its own nonce.
    let posted = sent(&network, "exchange");
    assert!(posted[1]["nonce"].as_u64() > posted[0]["nonce"].as_u64());
    assert!(matches!(
        posted[1]["signature"]["v"].as_u64(),
        Some(27 | 28)
    ));

    // The order shows among the open ones at once, and × cancels it.
    app.wait_for("id:trade-panel-order-1001").unwrap();
    assert!(app.shows("0.01 BTC") && app.shows("61013.0"));
    assert!(
        app.screen()
            .matches("id:trade-panel-orders >> text:1")
            .unwrap()
    );
    app.press("label:Cancel Buy 0.01 BTC at 61013.0").unwrap();
    app.wait_for("Cancelled: Buy 0.01 BTC at 61013.0.").unwrap();
    app.wait_for_gone("id:trade-panel-order-1001", WAIT)
        .unwrap();
    app.wait_for("No open orders").unwrap();
    assert_eq!(
        acted(&network)[1],
        json!({ "type": "cancel", "cancels": [{ "a": 0, "o": 1001 }] })
    );

    // A market order has no price of its own: it is sent as one that
    // cancels what does not fill at once, 5% past the mid.
    app.press("id:trade-panel-side-sell").unwrap();
    app.press("id:trade-panel-type-market").unwrap();
    assert!(!app.screen().matches(PRICE).unwrap());
    assert!(sends(&mut app, "Sell 0.01 BTC at market"));
    app.press(SEND).unwrap();
    app.press(SEND).unwrap();
    app.wait_for("Sold 0.01 BTC at 61000.0.").unwrap();
    let market = &acted(&network)[2]["orders"][0];
    assert_eq!(market["b"], false);
    // 61,012.7 less 5%, in five figures.
    assert_eq!(market["p"], "57962");
    assert_eq!(market["t"], json!({ "limit": { "tif": "Ioc" } }));

    // A size finer than the asset's is refused here, in words, and
    // nothing is sent for another size.
    app.press("id:trade-panel-type-limit").unwrap();
    app.clear(SIZE).unwrap();
    app.type_text(SIZE, "0.000001").unwrap();
    app.press(SEND).unwrap();
    app.press(SEND).unwrap();
    app.wait_for("A size of BTC has 5 decimal places at most.")
        .unwrap();
    assert_eq!(acted(&network).len(), 3);

    // What the exchange refuses is said in its own words.
    app.clear(SIZE).unwrap();
    app.type_text(SIZE, "0.0001").unwrap();
    app.press(SEND).unwrap();
    app.press(SEND).unwrap();
    app.wait_for("Order must have minimum value of $10.")
        .unwrap();
    assert_eq!(acted(&network).len(), 4);

    // And what is missing of an order, before anything is asked.
    app.clear(SIZE).unwrap();
    app.press(SEND).unwrap();
    app.wait_for("Enter a size above zero (1.5).").unwrap();
    assert_eq!(acted(&network).len(), 4);
}

#[test]
fn the_market_is_the_exchanges_and_the_settings_are_saved() {
    let (mut app, network) = start(Root::of::<Quick>());
    enable(&mut app);

    // The market as it was typed, traded as the exchange writes it.
    assert_eq!(app.screen().input_text(MARKET).unwrap(), Some("eth"));
    app.wait_for("1891.40").unwrap();
    assert!(sends(&mut app, "Buy ETH"));

    // With orders not confirmed, one press sends: ETH is the second asset.
    app.type_text(SIZE, "1").unwrap();
    app.type_text(PRICE, "1891.4").unwrap();
    assert!(sends(&mut app, "Buy 1 ETH at 1891.4"));
    app.press(SEND).unwrap();
    app.wait_for("Resting: Buy 1 ETH at 1891.4 (order 1001).")
        .unwrap();
    let order = &acted(&network)[0]["orders"][0];
    assert_eq!((&order["a"], &order["p"]), (&json!(1), &json!("1891.4")));

    // A price finer than the asset's is refused, not rounded.
    app.clear(PRICE).unwrap();
    app.type_text(PRICE, "1891.456").unwrap();
    app.press(SEND).unwrap();
    app.wait_for("A price of ETH has 5 significant figures and 2 decimal places at most.")
        .unwrap();
    assert_eq!(acted(&network).len(), 1);

    // What the exchange does not list is said, and cannot be sent for.
    app.clear(MARKET).unwrap();
    app.type_text(MARKET, "DOGE\n").unwrap();
    app.wait_for("Hyperliquid lists no DOGE market.").unwrap();
    // (Its cell takes no press.)
    drop(app.press(SEND));
    assert_eq!(acted(&network).len(), 1);
    app.clear(MARKET).unwrap();
    app.type_text(MARKET, "MATIC\n").unwrap();
    app.wait_for("MATIC is delisted on Hyperliquid.").unwrap();
    app.clear(MARKET).unwrap();
    app.type_text(MARKET, " kPEPE \n").unwrap();
    app.wait_for_gone("id:trade-panel-unlisted", WAIT).unwrap();
    assert_eq!(app.screen().input_text(MARKET).unwrap(), Some("kPEPE"));
    app.wait_for("0.01111").unwrap();
    assert_eq!(acted(&network).len(), 1);

    // Its settings: the exchange, the link color, and whether orders are
    // confirmed, which the cell that sends follows at once.
    app.press("Widget settings").unwrap();
    app.wait_for("id:trade-panel-settings").unwrap();
    for row in ["Exchange", "Link Color", "Confirm Orders"] {
        assert!(app.shows(row), "{row}");
    }
    app.press("id:trade-panel-confirm").unwrap();
    app.press("label:Close Trade Panel settings").unwrap();
    app.wait_for_gone("id:trade-panel-settings", WAIT).unwrap();
    app.clear(PRICE).unwrap();
    app.type_text(PRICE, "0.011").unwrap();
    app.clear(SIZE).unwrap();
    app.type_text(SIZE, "1500").unwrap();
    app.press(SEND).unwrap();
    assert!(sends(&mut app, "Confirm: Buy 1500 kPEPE at 0.011"));
    assert_eq!(acted(&network).len(), 1);
}

#[test]
fn another_account_in_the_wallet_has_enabled_nothing() {
    let (mut app, network) = start(Root::of::<Fresh>());
    enable(&mut app);
    app.wait_for("$1,234.50").unwrap();
    app.type_text(SIZE, "0.01").unwrap();
    app.type_text(PRICE, "61000").unwrap();
    app.press(SEND).unwrap();
    assert!(sends(&mut app, "Confirm: Buy 0.01 BTC at 61000"));

    // The wallet moves to another account: the agent the first approved
    // is dropped, the ticket gives way to enabling again, and the armed
    // order is never sent.
    app.press("the wallet changes account").unwrap();
    app.wait_for("id:trade-panel-ask").unwrap();
    assert!(app.shows("Trading off"));
    assert!(!app.screen().matches(SEND).unwrap());
    assert!(acted(&network).is_empty());

    // The stand-in signs with the first account's key whatever account it
    // says it is: an approval that is not the account's own is never
    // sent, and enables nothing.
    app.press(ENABLE).unwrap();
    app.wait_until("the other signer being named", WAIT, |screen| {
        screen.texts().iter().any(|text| {
            text.starts_with("Hyperliquid: Signed by 0x") && text.contains("not by this account")
        })
    })
    .unwrap();
    assert!(app.shows("Trading off"));
    assert!(!app.screen().matches(SEND).unwrap());
    assert_eq!(sent(&network, "exchange").len(), 1);
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
fn the_panel_is_blocks_that_meet_edge_to_edge() {
    let (mut app, _network) = start(Root::of::<Fresh>());
    enable(&mut app);
    app.wait_for("$1,234.50").unwrap();
    let left = |cell: Bounds<Points>| cell.origin.x.0;
    let right = |cell: Bounds<Points>| cell.origin.x.0 + cell.size.width.0;
    let meet = |a: f32, b: f32| (a - b).abs() < 0.5;
    let whole = placed(&app, "trade-panel");

    // The account's bar: its cells fill it side to side, each as high as
    // the bar inside its line, each beginning where the last ends.
    let bar = ["account", "equity", "available", "margin-ratio"]
        .map(|id| placed(&app, &format!("trade-panel-{id}")));
    assert!(meet(left(bar[0]), left(whole)) && meet(right(bar[3]), right(whole)));
    for pair in bar.windows(2) {
        assert!(meet(right(pair[0]), left(pair[1])), "{pair:?}");
        assert_eq!(pair[0].size.height, pair[1].size.height);
    }
    assert_eq!(bar[0].size.height, px(39.0));

    // Buy and Sell are two halves of their row.
    let [buy, sell] = ["buy", "sell"].map(|id| placed(&app, &format!("trade-panel-side-{id}")));
    assert!(meet(left(buy), left(whole)) && meet(right(sell), right(whole)));
    assert!(meet(right(buy), left(sell)));
    assert!((buy.size.width.0 - sell.size.width.0).abs() < 1.5);
    assert_eq!(buy.size.height, px(36.0));

    // A field keeps its height under its row's line, and the cell that
    // sends fills its row.
    for field in [
        "market",
        "leverage",
        "price",
        "size",
        "take-profit",
        "stop-loss",
    ] {
        let field = placed(&app, &format!("trade-panel-{field}"));
        assert_eq!(field.size.height, px(36.0), "{field:?}");
    }
    // The leverage's row: its field, then the margin mode, then the cell
    // that sets both, which ends at the pane's edge.
    let (leverage, set) = (
        placed(&app, "trade-panel-leverage"),
        placed(&app, "trade-panel-set-leverage"),
    );
    assert!(right(leverage) < left(set) && meet(right(set), right(whole)));
    assert_eq!(set.size.width, px(60.0));
    // An exit's row: its field, then what it would make, to the edge.
    let (take, made) = (
        placed(&app, "trade-panel-take-profit"),
        placed(&app, "trade-panel-take-profit-move"),
    );
    assert!(meet(right(take), left(made)) && meet(right(made), right(whole)));
    let send = placed(&app, "trade-panel-send");
    assert!(meet(left(send), left(whole)) && meet(right(send), right(whole)));
    assert_eq!(send.size.height, px(36.0));

    // Asked again, the cell makes room for × and nothing else moves.
    app.type_text(SIZE, "0.01").unwrap();
    app.type_text(PRICE, "61000").unwrap();
    app.press(SEND).unwrap();
    app.wait_for("id:trade-panel-disarm").unwrap();
    let (send, back) = (
        placed(&app, "trade-panel-send"),
        placed(&app, "trade-panel-disarm"),
    );
    assert!(meet(right(send), left(back)) && meet(right(back), right(whole)));
    assert_eq!(back.size.width, px(36.0));

    // A narrow pane leaves the last figure out, and the bar still fills it.
    let (mut app, _network) = start_in(Root::of::<Fresh>(), 400.0);
    enable(&mut app);
    app.wait_for("$1,234.50").unwrap();
    assert!(!app.screen().matches("id:trade-panel-margin-ratio").unwrap());
    let (whole, available) = (
        placed(&app, "trade-panel"),
        placed(&app, "trade-panel-available"),
    );
    assert!(whole.size.width.0 <= 400.0);
    assert!(meet(right(available), right(whole)));
    // The leverage's row still fits: a field to type in, and the cell
    // that sets at the pane's edge.
    let (leverage, set) = (
        placed(&app, "trade-panel-leverage"),
        placed(&app, "trade-panel-set-leverage"),
    );
    assert!(leverage.size.width.0 >= 60.0, "{leverage:?}");
    assert!(meet(right(set), right(whole)));
}

/// How many requests of kind the app has made of the stubbed Hyperliquid.
fn asked(network: &StubNetwork, kind: &str) -> usize {
    sent(network, "info")
        .iter()
        .filter(|body| body["type"] == kind)
        .count()
}

#[test]
fn what_the_market_does_to_the_account_shows_as_it_happens() {
    let Bench {
        mut app,
        network,
        exchange,
        ..
    } = start_live(Root::of::<Fresh>());
    enable(&mut app);
    app.wait_for("$1,234.50").unwrap();
    app.wait_for("No open orders").unwrap();

    // The account is followed: its three topics, each subscribed once (the
    // socket subscribes what is wanted in the order of its keys), beside
    // the market's price.
    let of_account = || -> Vec<Value> {
        let subscribed = exchange.subscribed().into_iter();
        subscribed
            .filter(|topic| topic.get("user").is_some())
            .collect()
    };
    app.wait_until("the account being followed", WAIT, |_| {
        of_account().len() >= 3
    })
    .unwrap();
    assert_eq!(
        of_account(),
        [
            json!({ "type": "clearinghouseState", "user": ACCOUNT }),
            json!({ "type": "openOrders", "user": ACCOUNT }),
            json!({ "type": "userFills", "user": ACCOUNT, "aggregateByTime": true }),
        ]
    );
    assert_eq!(exchange.socket.connections(), 1);

    // An order of the panel's own comes to rest.
    app.type_text(SIZE, "0.01").unwrap();
    app.type_text(PRICE, "61000").unwrap();
    app.press(SEND).unwrap();
    app.press(SEND).unwrap();
    app.wait_for("id:trade-panel-order-1001").unwrap();
    app.wait_for("Resting: Buy 0.01 BTC at 61000 (order 1001).")
        .unwrap();
    let polled = asked(&network, "frontendOpenOrders");

    // The market fills it: the order is gone from the open ones and the
    // fill is said, both at once, and no request was made to learn it.
    exchange.fill(1001);
    app.wait_for("Bought 0.01 BTC at 61000.0.").unwrap();
    app.wait_for_gone("id:trade-panel-order-1001", WAIT)
        .unwrap();
    app.wait_for("No open orders").unwrap();
    assert_eq!(asked(&network, "frontendOpenOrders"), polled);

    // An order placed somewhere else shows among the open ones, and × here
    // cancels it.
    exchange.rest(2001, "ETH", "A", "1900.0", "2.0");
    app.wait_for("id:trade-panel-order-2001").unwrap();
    assert!(app.shows("2 ETH") && app.shows("1900.00"));
    // What the account is worth follows its topic.
    exchange.worth("2000.0");
    app.wait_for("$2,000.00").unwrap();
    // A trade no order here made is the account's all the same.
    exchange.sold(3001, "ETH", "1891.4", "2.0");
    app.wait_for("Sold 2 ETH at 1891.40.").unwrap();
    assert_eq!(asked(&network, "frontendOpenOrders"), polled);

    // The connection is lost, and the order is cancelled elsewhere
    // meanwhile. The socket comes back, subscribes again, and the topics'
    // first frames say how things stand: the order is gone. The fills that
    // open the subscription are the account's history, not news: what the
    // panel last said stays.
    app.clear(SIZE).unwrap();
    app.press(SEND).unwrap();
    app.wait_for("Enter a size above zero (1.5).").unwrap();
    exchange.socket.sever();
    exchange.withdraw(2001);
    app.wait_until("the socket coming back", WAIT, |_| {
        exchange.socket.connections() == 2 && of_account().len() >= 6
    })
    .unwrap();
    app.wait_for_gone("id:trade-panel-order-2001", WAIT)
        .unwrap();
    assert!(app.shows("Enter a size above zero (1.5)."));
    assert!(!app.shows("Sold 2 ETH at 1891.40."));
}

#[test]
fn a_followed_account_is_polled_only_while_its_stream_is_silent() {
    let Bench {
        mut app,
        network,
        exchange,
        ..
    } = start_on(
        Root::of::<Fresh>(),
        1280.0,
        &Exchange::new(),
        BTreeMap::new(),
        Clock::manual(SystemTime::now()),
    );
    enable(&mut app);
    app.wait_for("$1,234.50").unwrap();
    app.wait_for("No open orders").unwrap();
    app.run_for(Duration::from_secs(1)).unwrap();
    let polls = |network: &StubNetwork| {
        (
            asked(network, "frontendOpenOrders"),
            asked(network, "clearinghouseState"),
        )
    };
    let before = polls(&network);

    // For a minute the stream says how the account stands every four
    // seconds, sooner than either request is due again (its orders every
    // twenty, its account every ten): neither is asked for.
    for _ in 0..15 {
        exchange.push_orders();
        exchange.push_account();
        app.run_for(Duration::from_secs(4)).unwrap();
    }
    assert_eq!(polls(&network), before);

    // The stream falls silent: the requests take over within their
    // intervals.
    app.run_for(Duration::from_secs(25)).unwrap();
    let (orders, account) = polls(&network);
    assert_eq!(orders, before.0 + 1);
    assert!(account >= before.1 + 2, "{account} after {}", before.1);
}

#[test]
fn trading_stays_enabled_between_runs_while_hyperliquid_lists_the_agent() {
    let exchange = Exchange::new();
    let run = |saved: BTreeMap<String, String>| {
        start_on(
            Root::of::<Fresh>(),
            1280.0,
            &exchange,
            saved,
            Clock::system(),
        )
    };

    // The first run enables trading: one approval, and the agent is kept
    // under its net and its account.
    let mut first = run(BTreeMap::new());
    enable(&mut first.app);
    let approved = sent(&first.network, "exchange");
    assert_eq!(approved.len(), 1);
    let agent = approved[0]["action"]["agentAddress"].clone();
    let saved = first.storage.entries();
    let kept: Value = serde_json::from_str(&saved[KEPT]).unwrap();
    assert_eq!(kept["Mainnet"][ACCOUNT]["agent"], agent);
    drop(first.app);

    // The next run: connecting the account is all it takes. Hyperliquid
    // is asked whether the agent still holds, the wallet is asked nothing,
    // and an order goes out signed as before.
    let mut second = run(saved.clone());
    second.app.wait_for("id:trade-panel-ask").unwrap();
    assert!(sent(&second.network, "info").is_empty());
    second.app.press("Connect wallet").unwrap();
    second.app.wait_for("id:trade-panel-account").unwrap();
    assert_eq!(asked(&second.network, "extraAgents"), 1);
    assert!(sent(&second.network, "exchange").is_empty());
    second.app.type_text(SIZE, "0.01").unwrap();
    second.app.type_text(PRICE, "61000").unwrap();
    second.app.press(SEND).unwrap();
    second.app.press(SEND).unwrap();
    second
        .app
        .wait_for("Resting: Buy 0.01 BTC at 61000 (order 1001).")
        .unwrap();
    assert_eq!(acted(&second.network).len(), 1);
    assert_eq!(second.storage.entries()[KEPT], saved[KEPT]);
    drop(second.app);

    // Trading is enabled on another device meanwhile, which retires this
    // one's agent on Hyperliquid. The run after finds it no longer listed:
    // nothing trades, the kept key is discarded, and enabling asks the
    // wallet again.
    exchange.book.lock().unwrap().agent =
        Some("0x5e9ee1089755c3435139848e47e6635505d5a13a".to_owned());
    let mut third = run(saved);
    third.app.wait_for("id:trade-panel-ask").unwrap();
    third.app.press("Connect wallet").unwrap();
    third.app.wait_for("Trading off").unwrap();
    assert_eq!(asked(&third.network, "extraAgents"), 1);
    assert!(!third.app.screen().matches(SEND).unwrap());
    assert!(!third.storage.entries().contains_key(KEPT));
    third.app.press(ENABLE).unwrap();
    third.app.wait_for("id:trade-panel-account").unwrap();
    let again = sent(&third.network, "exchange");
    assert_eq!(again.len(), 1);
    assert_ne!(again[0]["action"]["agentAddress"], agent);
    assert!(third.storage.entries().contains_key(KEPT));
}

const LEVERAGE: &str = "id:trade-panel-leverage";
const SET_LEVERAGE: &str = "id:trade-panel-set-leverage";

/// Waits for the field SELECTOR to hold TEXT.
fn holds(app: &mut Headless, selector: &str, text: &str) -> bool {
    let held = app.wait_until("the field's text", WAIT, |screen| {
        screen.input_text(selector).ok().flatten() == Some(text)
    });
    held.is_ok()
}

#[test]
fn the_leverage_and_the_margin_mode_are_shown_and_set() {
    let Bench {
        mut app,
        network,
        exchange,
        ..
    } = start_live(Root::of::<Fresh>());
    enable(&mut app);

    // The account's setting for the market, as the exchange has it, and
    // what the market takes at most where nothing is typed.
    assert!(holds(&mut app, LEVERAGE, "5"));
    assert!(
        app.screen()
            .matches("label:Margin mode >> text:Cross")
            .unwrap()
    );
    assert!(sent(&network, "info").iter().any(|body| {
        body["type"] == "activeAssetData" && body["user"] == ACCOUNT && body["coin"] == "BTC"
    }));
    // Nothing differs yet: there is nothing to set.
    drop(app.press(SET_LEVERAGE));
    assert!(acted(&network).is_empty());

    // Another leverage and the position's own margin, set as one action.
    app.clear(LEVERAGE).unwrap();
    app.type_text(LEVERAGE, "20").unwrap();
    app.press("label:Margin mode").unwrap();
    app.press("role:option[Isolated]").unwrap();
    app.press(SET_LEVERAGE).unwrap();
    app.wait_for("BTC set to 20x Isolated.").unwrap();
    assert_eq!(
        acted(&network),
        [json!({ "type": "updateLeverage", "asset": 0, "isCross": false, "leverage": 20 })]
    );
    // The setting is asked for again and is what shows; nothing differs.
    assert!(holds(&mut app, LEVERAGE, "20"));
    assert!(
        app.screen()
            .matches("label:Margin mode >> text:Isolated")
            .unwrap()
    );
    drop(app.press(SET_LEVERAGE));
    assert_eq!(acted(&network).len(), 1);

    // More than the market takes, and what is no leverage, are said here
    // and not sent.
    app.clear(LEVERAGE).unwrap();
    app.type_text(LEVERAGE, "41").unwrap();
    app.press(SET_LEVERAGE).unwrap();
    app.wait_for("Leverage of BTC is a whole number from 1 to 40.")
        .unwrap();
    app.clear(LEVERAGE).unwrap();
    app.type_text(LEVERAGE, "2.5").unwrap();
    app.press(SET_LEVERAGE).unwrap();
    app.wait_for("Enter leverage as a whole number (20).")
        .unwrap();
    assert_eq!(acted(&network).len(), 1);

    // What the exchange will not change while a position is open, it says
    // itself, and the setting stays what it is.
    exchange.hold("BTC", "0.01", "60000.0", "610.0", "10.0");
    app.wait_for("id:trade-panel-position").unwrap();
    app.clear(LEVERAGE).unwrap();
    app.type_text(LEVERAGE, "20").unwrap();
    app.press("label:Margin mode").unwrap();
    app.press("role:option[Cross]").unwrap();
    app.press(SET_LEVERAGE).unwrap();
    app.wait_for("Cannot switch leverage type with open position.")
        .unwrap();
    assert_eq!(acted(&network).len(), 2);

    // Another market has its own setting.
    app.clear(MARKET).unwrap();
    app.type_text(MARKET, "ETH\n").unwrap();
    assert!(holds(&mut app, LEVERAGE, "10"));
    assert!(
        app.screen()
            .matches("label:Margin mode >> text:Cross")
            .unwrap()
    );
}

#[test]
fn an_order_is_placed_with_its_take_profit_and_stop_loss() {
    let (mut app, network) = start(Root::of::<Fresh>());
    enable(&mut app);
    const TAKE: &str = "id:trade-panel-take-profit";
    const STOP: &str = "id:trade-panel-stop-loss";

    // Each exit says what the order would have made or lost there.
    app.type_text(SIZE, "0.01").unwrap();
    app.type_text(PRICE, "61000").unwrap();
    app.type_text(TAKE, "65000").unwrap();
    app.type_text(STOP, "58000").unwrap();
    app.wait_for("id:trade-panel-take-profit-move >> text:+6.56%")
        .unwrap();
    app.wait_for("id:trade-panel-stop-loss-move >> text:-4.92%")
        .unwrap();
    // What is asked about again names them.
    assert!(sends(&mut app, "Buy 0.01 BTC at 61000, TP 65000, SL 58000"));
    app.press(SEND).unwrap();
    assert!(sends(
        &mut app,
        "Confirm: Buy 0.01 BTC at 61000, TP 65000, SL 58000"
    ));
    app.press(SEND).unwrap();
    app.wait_for("Resting: Buy 0.01 BTC at 61000, TP 65000, SL 58000 (order 1001).")
        .unwrap();

    // One action: the order, then its exits, which sell its size once
    // triggered, no worse than a tenth past their triggers, and only
    // reduce.
    let exit = |tpsl: &str, trigger: &str, worst: &str| {
        json!({
            "a": 0, "b": false, "p": worst, "s": "0.01", "r": true,
            "t": { "trigger": { "isMarket": true, "triggerPx": trigger, "tpsl": tpsl } },
        })
    };
    assert_eq!(
        acted(&network),
        [json!({
            "type": "order",
            "orders": [
                { "a": 0, "b": true, "p": "61000", "s": "0.01", "r": false,
                  "t": { "limit": { "tif": "Gtc" } } },
                exit("tp", "65000", "58500"),
                exit("sl", "58000", "52200"),
            ],
            "grouping": "normalTpsl",
        })]
    );
    // The order lists with the exits that wait for it.
    app.wait_for("id:trade-panel-order-1001").unwrap();
    app.wait_for("Limit · TP 65000.0 · SL 58000.0").unwrap();

    // An exit on the wrong side of the order's price would trigger at
    // once: said here, and not sent.
    app.clear(TAKE).unwrap();
    app.type_text(TAKE, "60000").unwrap();
    app.press(SEND).unwrap();
    app.wait_for("A long's take profit is above its price.")
        .unwrap();
    assert_eq!(acted(&network).len(), 1);

    // An order that only reduces opens nothing to exit: the rows go, and
    // what was typed in them is not sent with it.
    app.press("id:trade-panel-reduce-only").unwrap();
    assert!(!app.screen().matches(TAKE).unwrap());
    assert!(!app.screen().matches(STOP).unwrap());
    assert!(sends(&mut app, "Buy 0.01 BTC at 61000"));
    app.press(SEND).unwrap();
    app.press(SEND).unwrap();
    app.wait_for("Resting: Buy 0.01 BTC at 61000 (order 1004).")
        .unwrap();
    let plain = &acted(&network)[1];
    assert_eq!(plain["grouping"], "na");
    assert_eq!(plain["orders"].as_array().unwrap().len(), 1);
    assert_eq!(plain["orders"][0]["r"], true);
}

#[test]
fn a_positions_exits_are_placed_for_the_whole_of_it() {
    let Bench {
        mut app,
        network,
        exchange,
        ..
    } = start_live(Root::of::<Fresh>());
    enable(&mut app);
    app.wait_for("No open orders").unwrap();
    const POSITION: &str = "id:trade-panel-position";
    const TAKE: &str = "id:trade-panel-position-take-profit";
    const STOP: &str = "id:trade-panel-position-stop-loss";
    const SET: &str = "id:trade-panel-position-set";

    // No position in the market: nothing to place exits for.
    assert!(!app.screen().matches(POSITION).unwrap());

    // The account comes to hold some: which way, how much, where from and
    // what it has made.
    exchange.hold("BTC", "0.01", "60000.0", "610.0", "10.0");
    app.wait_for(POSITION).unwrap();
    for shown in [
        "POSITION",
        "LONG",
        "0.01 BTC",
        "at 60000.0",
        "+$10.00 (+8.33%)",
    ] {
        assert!(
            app.screen()
                .matches(&format!("{POSITION} >> text:{shown}"))
                .unwrap(),
            "{shown}"
        );
    }

    // Nothing typed is nothing to place, and a stop on the wrong side of
    // the mark (61,000) would close the position at once: neither is sent.
    app.press(SET).unwrap();
    app.wait_for("Enter a take profit or a stop loss.").unwrap();
    app.type_text(STOP, "62000").unwrap();
    app.press(SET).unwrap();
    app.wait_for("A long's stop loss is below its price.")
        .unwrap();
    assert!(acted(&network).is_empty());

    // Both, placed as one action: they sell once triggered, have no size
    // of their own (the position's, whatever it is then), and only reduce.
    app.clear(STOP).unwrap();
    app.type_text(STOP, "58000").unwrap();
    app.type_text(TAKE, "65000").unwrap();
    app.press(SET).unwrap();
    app.wait_for("Take profit at 65000 and stop loss at 58000 placed for the BTC position.")
        .unwrap();
    let exit = |tpsl: &str, trigger: &str, worst: &str| {
        json!({
            "a": 0, "b": false, "p": worst, "s": "0", "r": true,
            "t": { "trigger": { "isMarket": true, "triggerPx": trigger, "tpsl": tpsl } },
        })
    };
    assert_eq!(
        acted(&network),
        [json!({
            "type": "order",
            "orders": [exit("tp", "65000", "58500"), exit("sl", "58000", "52200")],
            "grouping": "positionTpsl",
        })]
    );
    // The fields are empty for the next, and the exits are open orders:
    // each known by the price it waits for, and for the position.
    assert!(holds(&mut app, TAKE, "") && holds(&mut app, STOP, ""));
    app.wait_for("id:trade-panel-order-1001").unwrap();
    app.wait_for("id:trade-panel-order-1002").unwrap();
    for shown in [
        "BTC position",
        "65000.0",
        "Take Profit Market",
        "58000.0",
        "Stop Market",
    ] {
        assert!(
            app.screen()
                .matches(&format!("id:trade-panel-orders >> text:{shown}"))
                .unwrap(),
            "{shown}"
        );
    }

    // Each is cancelled as an order is.
    app.press("label:Cancel Sell BTC position at 58000.0")
        .unwrap();
    app.wait_for("Cancelled: Sell BTC position at 58000.0.")
        .unwrap();
    app.wait_for_gone("id:trade-panel-order-1002", WAIT)
        .unwrap();
    assert_eq!(
        acted(&network)[1],
        json!({ "type": "cancel", "cancels": [{ "a": 0, "o": 1002 }] })
    );

    // The position is closed: there is nothing to place exits for.
    exchange.flat();
    app.wait_for_gone(POSITION, WAIT).unwrap();
}

/// A Trade Panel on Paper, an exchange that is none and has nothing a
/// venue may leave out, trading a market typed in lower case.
#[component]
fn OnPaper() -> impl IntoElement {
    let saved = json!({ "venue": "paper", "instrument": "btc-perp" });
    trade_bench::workspace_with_paper(WidgetKind::TradePanel, Some(saved))
}

#[test]
fn the_panel_trades_on_an_exchange_that_has_only_what_a_venue_must() {
    let Bench {
        mut app,
        network,
        exchange,
        ..
    } = start_live(Root::of::<OnPaper>());

    // The same way in: a wallet, then this exchange enabled for its
    // account, by the exchange's own rows.
    app.wait_for("id:trade-panel-ask").unwrap();
    app.press("Connect wallet").unwrap();
    app.wait_for("Trading off").unwrap();
    assert!(app.shows("Paper trades nothing real."));
    app.press("label:Enable trading on Paper").unwrap();
    app.wait_for("id:trade-panel-account").unwrap();
    assert!(app.shows(ACCOUNT_SHORT));
    // Enabled there, and nowhere else.
    assert!(app.shows("paper is Ready") && app.shows("hyperliquid is Locked"));

    // The market as the exchange writes it, whatever shape its names have.
    app.wait_for("No open orders").unwrap();
    assert!(sends(&mut app, "Buy BTC-PERP"));
    // What the exchange does not have is not there: no figures of a margin
    // account it reports none of, no price nothing streams, no row for a
    // leverage it has no setting of, no position block.
    assert!(
        app.screen()
            .matches(&format!("id:trade-panel-equity >> text:{NONE}"))
            .unwrap()
    );
    assert!(
        app.screen()
            .matches(&format!("id:trade-panel-mid >> text:{NONE}"))
            .unwrap()
    );
    assert!(!app.screen().enabled("id:trade-panel-use-mid").unwrap());
    assert!(!app.screen().matches(LEVERAGE).unwrap());
    assert!(!app.screen().matches(SET_LEVERAGE).unwrap());
    assert!(!app.screen().matches("id:trade-panel-position").unwrap());
    assert!(
        !app.screen()
            .matches("id:trade-panel-account-problem")
            .unwrap()
    );

    // A limit order is confirmed, placed, listed and cancelled.
    app.type_text(SIZE, "1").unwrap();
    app.type_text(PRICE, "100").unwrap();
    app.press(SEND).unwrap();
    assert!(sends(&mut app, "Confirm: Buy 1 BTC-PERP at 100"));
    app.press(SEND).unwrap();
    app.wait_for("Resting: Buy 1 BTC-PERP at 100 (order 1).")
        .unwrap();
    app.wait_for("id:trade-panel-order-1").unwrap();
    app.press("label:Cancel Buy 1 BTC-PERP at 100.000").unwrap();
    app.wait_for("Cancelled: Buy 1 BTC-PERP at 100.000.")
        .unwrap();
    app.wait_for("No open orders").unwrap();

    // What the exchange refuses is said in its words, and an order it
    // takes with nothing more to say is said to be taken.
    app.clear(SIZE).unwrap();
    app.type_text(SIZE, "0.25").unwrap();
    app.press(SEND).unwrap();
    app.press(SEND).unwrap();
    app.wait_for("A size of BTC-PERP has 1 decimal place at most.")
        .unwrap();
    app.clear(SIZE).unwrap();
    app.type_text(SIZE, "1").unwrap();
    app.press("id:trade-panel-type-market").unwrap();
    app.press(SEND).unwrap();
    app.press(SEND).unwrap();
    app.wait_for("Accepted: Buy 1 BTC-PERP at market.").unwrap();

    // A market it no longer takes orders for, and one it never listed.
    app.clear(MARKET).unwrap();
    app.type_text(MARKET, "old-perp\n").unwrap();
    app.wait_for("OLD-PERP is delisted on Paper.").unwrap();
    app.clear(MARKET).unwrap();
    app.type_text(MARKET, "BTC\n").unwrap();
    app.wait_for("Paper lists no BTC market.").unwrap();

    // None of it asked Hyperliquid anything, over either of its ways.
    assert!(sent(&network, "info").is_empty());
    assert!(sent(&network, "exchange").is_empty());
    assert!(exchange.socket.sent().is_empty());
}
