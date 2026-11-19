use std::collections::HashMap;

use nsh_core::{element::Root, prelude::*};
use nsh_testing::WAIT;
use serde_json::json;

use super::{
    session::{self, Figure, Health, Overview, Part, Reports},
    settings::AccountSummarySettings,
};
use crate::{
    data::{
        portfolio::{
            Account, Balance, Balances, Direction, Fees, Leverage, MarginAccount, MarginMode,
            Performance, Period, Point, Position, Prices, Scope, Staking, VaultShare, Vaults,
            Window,
        },
        Portfolio,
    },
    widgets::{
        common::accounts::{
            testing::{
                account, accounts, asked, asked_lighter, count, lighter_account, on, start,
                workspace_of, IDLE, TRADER, USER, WHALE,
            },
            AccountSettings, Mark, Tone, NONE,
        },
        WidgetKind,
    },
};

#[test]
fn settings_save_only_what_differs_and_read_leniently() {
    // Defaults save nothing: no account, the balances shown, none hidden.
    let defaults = AccountSummarySettings::default();
    assert_eq!(serde_json::to_value(&defaults).unwrap(), json!({}));
    assert!(defaults.balances() && !defaults.small_hidden() && defaults.accounts().is_empty());
    assert_eq!(
        AccountSummarySettings::from(json!({ "showBalances": true, "hideSmallBalances": false })),
        defaults
    );

    // The account and the switches; a switch of the wrong type is its
    // default.
    let mut saved = account(USER);
    saved["showBalances"] = json!(false);
    saved["hideSmallBalances"] = json!(true);
    let settings = AccountSummarySettings::from(saved.clone());
    assert!(!settings.balances() && settings.small_hidden());
    assert_eq!(settings.accounts()[0].address, USER);
    assert_eq!(serde_json::to_value(&settings).unwrap(), saved);
    assert_eq!(
        serde_json::to_value(settings.with_balances(true).with_small_hidden(false)).unwrap(),
        account(USER)
    );
    let damaged = AccountSummarySettings::from(json!({
        "showBalances": "no", "hideSmallBalances": "yes"
    }));
    assert!(damaged.balances() && !damaged.small_hidden());
    assert_eq!(AccountSummarySettings::from(json!(null)), defaults);
}

fn position(instrument: &str, direction: Direction, value: f64) -> Position {
    Position {
        instrument: instrument.to_owned(),
        direction,
        size: 1.0,
        entry_price: None,
        mark_price: None,
        value: Some(value),
        unrealized_pnl: None,
        return_on_equity: None,
        liquidation_price: None,
        margin_used: None,
        leverage: None,
    }
}

fn point(time_ms: f64, value: f64) -> Point {
    Point { time_ms, value }
}

fn window(period: Period, value: [f64; 2], pnl: f64) -> Window {
    Window {
        period,
        scope: Scope::Total,
        value: vec![point(0.0, value[0]), point(1.0, value[1])],
        pnl: vec![point(0.0, 0.0), point(1.0, pnl)],
        volume: Some(1_500.0),
    }
}

/// An account with every report, and the figures it reads as.
#[test]
fn the_overview_reads_every_report_and_dashes_what_is_not_reported() {
    let margin = MarginAccount {
        equity: Some(10_000.0),
        unrealized_pnl: Some(-99.9866),
        position_value: Some(8_000.0),
        margin_used: Some(2_500.0),
        maintenance_margin: Some(600.0),
        withdrawable: Some(7_500.0),
        cross_equity: Some(9_000.0),
        funding: Some(-12.5),
        positions: vec![
            position("BTC", Direction::Long, 4_800.0),
            position("ETH", Direction::Long, 1_200.0),
            Position {
                leverage: Some(Leverage {
                    value: 5.0,
                    mode: MarginMode::Isolated,
                }),
                margin_used: Some(400.0),
                ..position("SOL", Direction::Short, 2_000.0)
            },
        ],
        ..MarginAccount::default()
    };
    let balances = Balances(vec![
        Balance {
            asset: "USDC".to_owned(),
            total: 500.0,
            held: 0.0,
            entry_value: None,
        },
        Balance {
            asset: "HYPE".to_owned(),
            total: 25.0,
            held: 0.0,
            entry_value: Some(800.0),
        },
    ]);
    let prices = Prices(HashMap::from([
        ("USDC".to_owned(), 1.0),
        ("HYPE".to_owned(), 40.0),
    ]));
    let performance = Performance(vec![
        window(Period::Day, [14_000.0, 15_000.0], 140.0),
        window(Period::Week, [13_000.0, 15_000.0], -260.0),
        window(Period::Month, [12_000.0, 15_000.0], 2_450.0),
    ]);
    let staking = Staking {
        asset: "HYPE".to_owned(),
        delegated: 50.0,
        undelegated: 0.0,
        pending_withdrawal: 0.0,
    };
    let vaults = Vaults(vec![VaultShare {
        vault: "0xabc".to_owned(),
        equity: 1_500.0,
        locked_until_ms: None,
    }]);
    let fees = Fees {
        taker: Some(0.000_36),
        maker: Some(0.000_108),
        spot_taker: Some(0.000_54),
        spot_maker: Some(0.000_27),
        volume: Some(15_000_000.0),
        volume_days: 14,
        tier: Some(1),
        tier_floor: Some(5_000_000.0),
        next_tier: Some(25_000_000.0),
        discount: Some(0.1),
    };
    let overview = Overview::of(Reports {
        margin: Some(&margin),
        balances: Some(&balances),
        prices: Some(&prices),
        performance: Some(&performance),
        staking: Some(&staking),
        vaults: Some(&vaults),
        fees: Some(&fees),
    });
    let read = |figure: Figure| overview.figure(figure).text.as_str();
    for (figure, expected) in [
        // The exchange's own total, and the day's change from its start.
        (Figure::Total, "$15,000.00"),
        (Figure::DayChange, "+$140.00"),
        (Figure::DayReturn, "+1.00%"),
        (Figure::MarginRatio, "6.67%"),
        (Figure::Buffer, "$8,400.00"),
        (Figure::Maintenance, "$600.00"),
        (Figure::CrossEquity, "$9,000.00"),
        (Figure::Equity, "$10,000.00"),
        (Figure::MarginUsed, "$2,500.00"),
        (Figure::Withdrawable, "$7,500.00"),
        (Figure::Leverage, "0.80x"),
        (Figure::Isolated, "1"),
        (Figure::IsolatedMargin, "$400.00"),
        (Figure::Long, "$6,000.00"),
        (Figure::Short, "$2,000.00"),
        (Figure::Net, "+$4,000.00"),
        (Figure::Gross, "$8,000.00"),
        (Figure::Positions, "3"),
        (Figure::Largest, "BTC · 60%"),
        (Figure::Unrealized, "-$99.99"),
        (Figure::Funding, "-$12.50"),
        (Figure::WeekPnl, "-$260.00"),
        (Figure::MonthPnl, "+$2,450.00"),
        (Figure::DayVolume, "$1,500.00"),
        (Figure::FeeTier, "Tier 1"),
        (Figure::FeeVolume, "$15,000,000.00"),
        (Figure::NextTier, "Tier 2 at $25M"),
        (Figure::PerpRates, "0.036% / 0.0108%"),
        (Figure::SpotRates, "0.054% / 0.027%"),
        (Figure::Discount, "10.0%"),
        (Figure::Staked, "50 HYPE"),
        (Figure::StakedValue, "$2,000.00"),
        (Figure::Undelegated, "0 HYPE"),
        (Figure::Unstaking, "0 HYPE"),
        (Figure::VaultEquity, "$1,500.00"),
        (Figure::VaultCount, "1"),
    ] {
        assert_eq!(read(figure), expected, "{figure:?}");
    }
    // Gains and losses are toned, and nothing else.
    assert_eq!(overview.figure(Figure::DayChange).tone, Some(Tone::Gain));
    assert_eq!(overview.figure(Figure::WeekPnl).tone, Some(Tone::Loss));
    assert_eq!(overview.figure(Figure::Funding).tone, Some(Tone::Loss));
    assert_eq!(overview.figure(Figure::Net).tone, None);

    // The bars: margin 1/15 of the cross equity, a quarter of the equity
    // held, three quarters long, half way from tier 1 to tier 2.
    assert!((overview.margin_ratio.unwrap() - 600.0 / 9_000.0).abs() < 1e-12);
    assert_eq!(Health::of(overview.margin_ratio.unwrap()), Health::Safe);
    assert_eq!(overview.margin_usage, Some(0.25));
    assert_eq!(overview.sides, Some((0.75, 0.25)));
    assert_eq!(overview.tier_progress, Some(0.5));

    // The parts: perps 10,000, spot 500 + 25 × 40, staked 50 × 40, vaults
    // 1,500, and what the exchange counts besides (15,000 less 15,000).
    let share = |part: Part| overview.share(part).unwrap();
    assert_eq!(share(Part::Perps).value, "$10,000.00");
    assert_eq!(share(Part::Spot).value, "$1,500.00");
    assert_eq!(share(Part::Staked).value, "$2,000.00");
    assert_eq!(share(Part::Vaults).value, "$1,500.00");
    assert_eq!(share(Part::Other).fraction, 0.0, "nothing uncounted");
    assert_eq!(share(Part::Perps).percent, "66.7%");

    // More reported than counted is another part.
    let mut more = performance.clone();
    more.0[0].value[1].value = 20_000.0;
    let other = Overview::of(Reports {
        margin: Some(&margin),
        balances: Some(&balances),
        prices: Some(&prices),
        performance: Some(&more),
        staking: Some(&staking),
        vaults: Some(&vaults),
        fees: Some(&fees),
    });
    assert_eq!(other.share(Part::Other).unwrap().value, "$5,000.00");
    assert_eq!(other.share(Part::Other).unwrap().percent, "25.0%");

    // No reports: every figure a dash, no bar.
    let waiting = Overview::of(Reports::default());
    assert!(Figure::ALL
        .iter()
        .all(|figure| waiting.figure(*figure).text == NONE));
    assert_eq!(
        (waiting.margin_ratio, waiting.sides, waiting.tier_progress),
        (None, None, None)
    );

    // Without the history the total is the parts together.
    let counted = Overview::of(Reports {
        margin: Some(&margin),
        balances: Some(&balances),
        prices: Some(&prices),
        ..Reports::default()
    });
    assert_eq!(counted.figure(Figure::Total).text, "$11,500.00");
    assert_eq!(counted.figure(Figure::DayChange).text, NONE);

    // Health by the ratio; the top tier is a full bar.
    assert_eq!(
        [0.1, 0.5, 0.79, 0.8, 1.2].map(Health::of),
        [
            Health::Safe,
            Health::Caution,
            Health::Caution,
            Health::Danger,
            Health::Danger
        ]
    );
    let top = Fees {
        tier: Some(6),
        next_tier: None,
        ..fees
    };
    let at_top = Overview::of(Reports {
        fees: Some(&top),
        ..Reports::default()
    });
    assert_eq!(at_top.tier_progress, Some(1.0));
    assert_eq!(at_top.figure(Figure::NextTier).text, "Top tier");

    // Rates drop their trailing zeros, and none is `0%`.
    let free = Fees {
        taker: Some(0.0003),
        maker: Some(0.0),
        spot_taker: Some(0.000_25),
        spot_maker: Some(-0.000_01),
        ..fees
    };
    let rated = Overview::of(Reports {
        fees: Some(&free),
        ..Reports::default()
    });
    assert_eq!(rated.figure(Figure::PerpRates).text, "0.03% / 0%");
    assert_eq!(rated.figure(Figure::SpotRates).text, "0.025% / -0.001%");
}

#[test]
fn several_accounts_make_one_overview() {
    let balance = |asset: &str, total: f64| Balance {
        asset: asset.to_owned(),
        total,
        held: 0.0,
        entry_value: None,
    };
    let staking = |asset: &str, delegated: f64| Staking {
        asset: asset.to_owned(),
        delegated,
        undelegated: 1.0,
        pending_withdrawal: 0.0,
    };
    let fees = Fees {
        tier: Some(1),
        volume: Some(1_000.0),
        ..Fees::default()
    };
    // One at ease on an exchange that reports everything.
    let eased = MarginAccount {
        equity: Some(10_000.0),
        unrealized_pnl: Some(100.0),
        position_value: Some(4_000.0),
        margin_used: Some(1_000.0),
        maintenance_margin: Some(500.0),
        withdrawable: Some(9_000.0),
        cross_equity: Some(10_000.0),
        funding: Some(1.0),
        positions: vec![position("BTC", Direction::Long, 4_000.0)],
        ..MarginAccount::default()
    };
    let (held, priced) = (
        Balances(vec![balance("USDC", 500.0), balance("TOK", 10.0)]),
        Prices(HashMap::from([
            ("USDC".to_owned(), 1.0),
            ("TOK".to_owned(), 2.0),
        ])),
    );
    let (staked, history) = (
        staking("TOK", 5.0),
        Performance(vec![window(Period::Day, [9_000.0, 99_999.0], 140.0)]),
    );
    // One near liquidation on another, which prices the token its own way
    // and reports no fees.
    let strained = MarginAccount {
        equity: Some(2_000.0),
        unrealized_pnl: Some(-50.0),
        position_value: Some(6_000.0),
        margin_used: Some(1_200.0),
        maintenance_margin: Some(1_500.0),
        withdrawable: Some(100.0),
        cross_equity: Some(2_000.0),
        funding: Some(-3.0),
        positions: vec![position("BTC", Direction::Short, 6_000.0)],
        ..MarginAccount::default()
    };
    let (also_held, also_priced, also_staked) = (
        Balances(vec![balance("TOK", 10.0)]),
        Prices(HashMap::from([("TOK".to_owned(), 3.0)])),
        staking("LIT", 7.0),
    );
    let first = Reports {
        margin: Some(&eased),
        balances: Some(&held),
        prices: Some(&priced),
        performance: Some(&history),
        staking: Some(&staked),
        fees: Some(&fees),
        ..Reports::default()
    };
    let second = Reports {
        margin: Some(&strained),
        balances: Some(&also_held),
        prices: Some(&also_priced),
        staking: Some(&also_staked),
        ..Reports::default()
    };
    let overview = Overview::of_accounts(&[first, second]);
    let read = |figure: Figure| overview.figure(figure).text.as_str();
    for (figure, expected) in [
        // What they are worth adds up.
        (Figure::Equity, "$12,000.00"),
        (Figure::CrossEquity, "$12,000.00"),
        (Figure::Maintenance, "$2,000.00"),
        (Figure::Buffer, "$10,000.00"),
        (Figure::MarginUsed, "$2,200.00"),
        (Figure::Withdrawable, "$9,100.00"),
        (Figure::Leverage, "0.83x"),
        // The ratio is the one nearest liquidation: 1,500 of 2,000.
        (Figure::MarginRatio, "75.00%"),
        // Exposure is of every position.
        (Figure::Long, "$4,000.00"),
        (Figure::Short, "$6,000.00"),
        (Figure::Net, "-$2,000.00"),
        (Figure::Positions, "2"),
        (Figure::Largest, "BTC · 60%"),
        (Figure::Unrealized, "+$50.00"),
        (Figure::Funding, "-$2.00"),
        // Each stake in its own token; the one its exchange does not price
        // is worth nothing known.
        (Figure::Staked, "5 TOK + 7 LIT"),
        (Figure::Undelegated, "1 TOK + 1 LIT"),
        (Figure::StakedValue, "$12.00"),
        // The one fee schedule reported.
        (Figure::FeeTier, "Tier 1"),
        // One of them has no history: what both made is not said, and the
        // whole is what is counted (12,000 + 550 + 12).
        (Figure::DayChange, NONE),
        (Figure::Total, "$12,562.00"),
    ] {
        assert_eq!(read(figure), expected, "{figure:?}");
    }
    // Each holding at its own exchange's price: 500 + 10 × 2, and 10 × 3.
    assert_eq!(overview.share(Part::Spot).unwrap().value, "$550.00");
    assert_eq!(overview.margin_ratio, Some(0.75));
    assert_eq!(Health::of(overview.margin_ratio.unwrap()), Health::Caution);
    assert_eq!(overview.sides, Some((0.4, 0.6)));

    // With a history of each, the whole is theirs together; with a fee
    // schedule of each, neither is the accounts'.
    let second = Reports {
        performance: Some(&history),
        fees: Some(&fees),
        ..second
    };
    let both = Overview::of_accounts(&[first, second]);
    assert_eq!(both.figure(Figure::Total).text, "$199,998.00");
    assert_eq!(both.figure(Figure::DayChange).text, "+$280.00");
    assert_eq!(both.figure(Figure::FeeTier).text, NONE);
    assert_eq!(both.figure(Figure::FeeVolume).text, NONE);

    // Their holdings are one table, each marked with its account, the most
    // valuable first: an asset both hold is a row of each.
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
    let holdings = session::holdings_of_all(
        &[
            (Some(mark("hyperliquid")), &held, Some(&priced)),
            (Some(mark("lighterxyz")), &also_held, Some(&also_priced)),
        ],
        false,
    );
    let listed: Vec<(&str, &str, &str)> = holdings
        .iter()
        .map(|holding| {
            (
                holding.owner.as_ref().unwrap().named.as_str(),
                holding.asset.as_str(),
                holding.value.as_str(),
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            ("Hyperliquid 0x0123…4567", "USDC", "$500.00"),
            ("Lighter 0x0123…4567", "TOK", "$30.00"),
            ("Hyperliquid 0x0123…4567", "TOK", "$20.00"),
        ]
    );
    assert_ne!(holdings[1].key, holdings[2].key);
}

#[test]
fn holdings_are_valued_and_the_most_valuable_come_first() {
    let balance = |asset: &str, total: f64, held: f64, entry_value: Option<f64>| Balance {
        asset: asset.to_owned(),
        total,
        held,
        entry_value,
    };
    let balances = Balances(vec![
        balance("USDC", 14.625_485, 4.5, Some(0.0)),
        balance("PURR", 2_000.0, 0.0, Some(1_234.56)),
        balance("USDC", 1.0, 0.0, None),
        balance("DUST", 0.000_012_3, 0.0, None),
        balance("CRUMB", 3.0, 0.0, None),
        balance("+1230", 25.0, 0.0, None),
    ]);
    let prices = Prices(HashMap::from([
        ("USDC".to_owned(), 1.0),
        ("PURR".to_owned(), 0.5),
        ("CRUMB".to_owned(), 0.1),
    ]));
    let holdings = session::holdings_of(&balances, Some(&prices), false);
    // The priced by worth, an asset once; then the rest in the exchange's
    // order.
    let assets: Vec<&str> = holdings.iter().map(|holding| &*holding.key).collect();
    assert_eq!(assets, ["PURR", "USDC", "CRUMB", "DUST", "+1230"]);
    let purr = &holdings[0];
    assert_eq!(
        [
            &purr.total,
            &purr.available,
            &purr.value,
            &purr.cost,
            &purr.pnl
        ]
        .map(String::as_str),
        ["2,000", "2,000", "$1,000.00", "$1,234.56", "-$234.56"]
    );
    assert_eq!(purr.tone, Some(Tone::Loss));
    let usdc = &holdings[1];
    assert_eq!(
        [
            &usdc.total,
            &usdc.available,
            &usdc.value,
            &usdc.cost,
            &usdc.pnl
        ]
        .map(String::as_str),
        ["14.6255", "10.1255", "$14.63", "$0.00", NONE]
    );
    assert_eq!(
        (holdings[3].value.as_str(), holdings[3].cost.as_str()),
        (NONE, NONE)
    );

    // Hiding small balances leaves out what is priced under a dollar only.
    let kept = session::holdings_of(&balances, Some(&prices), true);
    let assets: Vec<&str> = kept.iter().map(|holding| &*holding.key).collect();
    assert_eq!(assets, ["PURR", "USDC", "DUST", "+1230"]);

    // No prices: no worth, the exchange's order.
    let unpriced = session::holdings_of(&balances, None, true);
    let assets: Vec<&str> = unpriced.iter().map(|holding| &*holding.key).collect();
    assert_eq!(assets, ["USDC", "PURR", "DUST", "CRUMB", "+1230"]);
}

/// A workspace of one Account Summary watching the stubbed account.
#[component]
fn Watching() -> impl IntoElement {
    workspace_of(WidgetKind::AccountSummary, Some(account(USER)))
}

/// One watching an account that never traded.
#[component]
fn Idle() -> impl IntoElement {
    workspace_of(WidgetKind::AccountSummary, Some(account(IDLE)))
}

/// Whether the row of the figure `id` reads `read`.
fn reads(app: &nsh_testing::Headless, id: &str, read: &str) -> bool {
    app.screen()
        .matches(&format!("id:account-summary-{id} >> text:{read}"))
        .unwrap()
}

#[test]
fn the_summary_shows_the_account_s_standing_and_its_balances() {
    let (mut app, network) = start(Root::of::<Watching>());

    // A figure of each report, once it has arrived.
    for arrived in ["$13,109.48", "+$58.00", "Tier 1", "$520.00", "PURR"] {
        app.wait_for(arrived).unwrap();
    }
    for (id, read) in [
        // The headline: the history's total and its day.
        ("total", "$13,058.00"),
        ("day-change", "+$58.00"),
        ("day-return", "+0.45%"),
        // Margin health: the cross equity against its maintenance.
        ("margin-ratio", "0.47%"),
        ("liquidation-buffer", "$13,043.51"),
        ("maintenance-margin", "$61.00"),
        ("cross-equity", "$13,104.51"),
        ("equity", "$13,109.48"),
        ("margin-used", "$1,224.97"),
        ("withdrawable", "$11,884.51"),
        ("leverage", "0.47x"),
        // The isolated ETH long, with its own margin.
        ("isolated-positions", "1"),
        ("isolated-margin", "$4.97"),
        // Exposure: the ETH long against the BTC short.
        ("long", "$100.03"),
        ("short", "$6,100.00"),
        ("net-exposure", "-$5,999.97"),
        ("position-value", "$6,200.03"),
        ("open-positions", "2"),
        ("largest", "BTC · 98%"),
        ("unrealized-pnl", "+$99.99"),
        ("funding", "+$12.50"),
        ("week-pnl", "+$390.00"),
        ("month-pnl", "+$2,450.00"),
        ("day-volume", "$250,000.50"),
        ("fee-tier", "Tier 1"),
        ("fee-volume", "$7,000,000.00"),
        ("next-tier", "Tier 2 at $25M"),
        ("perp-rates", "0.036% / 0.0108%"),
        ("discount", "10.0%"),
        // 13 HYPE at 40, and the vault.
        ("staked", "10 HYPE"),
        ("staked-value", "$520.00"),
        ("vault-equity", "$1,000.00"),
        ("vault-count", "1"),
        // The parts: the perps, the tokens at their prices.
        ("part-perps", "$13,109.48"),
        ("part-spot", "$1,014.63"),
    ] {
        assert!(reads(&app, id, read), "{id}: {read}");
    }
    for name in [
        "TOTAL VALUE",
        "MARGIN HEALTH",
        "EXPOSURE",
        "FEES",
        "STAKING & VAULTS",
        "Liquidation Buffer",
    ] {
        assert!(app.shows(name), "{name}");
    }

    // Its balances, the most valuable first; the one it holds none of is
    // left out.
    for shown in ["USDC", "14.6255", "10.1255", "$14.63", "-$234.56", "COST"] {
        assert!(app.shows(shown), "{shown}");
    }
    assert!(!app.shows("No balances"));

    // Its figures, their names and the balances select and copy together.
    for id in ["margin-used", "total", "holdings"] {
        let selector = format!("id:account-summary-{id}");
        assert!(app.screen().in_selectable_view(&selector).unwrap(), "{id}");
    }

    // Every report is asked for, the prices once for the venue.
    let requests = asked(&network);
    for kind in [
        "clearinghouseState",
        "spotClearinghouseState",
        "portfolio",
        "delegatorSummary",
        "userVaultEquities",
        "userFees",
    ] {
        assert!(count(&requests, kind, USER) >= 1, "{kind}");
    }
    assert!(requests
        .iter()
        .any(|body| body["type"] == "spotMetaAndAssetCtxs"));

    // Show Balances takes them away, and brings them back.
    app.press("Widget settings").unwrap();
    app.wait_for("id:account-summary-settings").unwrap();
    app.press("label:Show Balances").unwrap();
    app.wait_for_gone("-$234.56", WAIT).unwrap();
    assert!(app.shows("$13,109.48"), "the figures stay");
    app.press("label:Show Balances").unwrap();
    app.wait_for("-$234.56").unwrap();
    app.press("label:Close Account Summary settings").unwrap();
    app.wait_for_gone("id:account-summary-settings", WAIT)
        .unwrap();
}

/// Where a view with a test id is laid out: its origin and its width.
fn place(app: &nsh_testing::Headless, test_id: &str) -> (f32, f32, f32) {
    let node = app
        .screen()
        .layout_nodes()
        .into_iter()
        .find(|node| node.test_id.as_deref() == Some(test_id))
        .unwrap_or_else(|| panic!("no view `{test_id}`"));
    (
        node.bounds.origin.x.0,
        node.bounds.origin.y.0,
        node.bounds.size.width.0,
    )
}

#[test]
fn tiles_share_a_wide_pane_and_the_bar_splits_by_worth() {
    let (mut app, _network) = start(Root::of::<Watching>());
    for arrived in ["+$58.00", "Tier 1", "$520.00", "$1,014.63"] {
        app.wait_for(arrived).unwrap();
    }
    // Side by side in a wide pane: the first two tiles on one line.
    let health = place(&app, "account-summary-tile-margin-health");
    let margin = place(&app, "account-summary-tile-margin");
    assert_eq!(health.1, margin.1, "one line");
    assert!(
        margin.0 > health.0 && health.2 >= 280.0,
        "{health:?} {margin:?}"
    );

    // Each part of the bar is as wide as its share of the whole: here the
    // parts together (15,644.11), more than the history's total.
    let (_, _, bar) = place(&app, "account-summary-parts");
    let (_, _, perps) = place(&app, "account-summary-segment-perps");
    let share = 13_109.482_328 / 15_644.107_813;
    assert!(
        (perps / bar - share as f32).abs() < 0.01,
        "{perps} of {bar}"
    );
    let (_, _, other) = place(&app, "account-summary-segment-other");
    assert_eq!(other, 0.0, "nothing uncounted");
}

/// One watching an account worth millions.
#[component]
fn Whale() -> impl IntoElement {
    workspace_of(WidgetKind::AccountSummary, Some(account(WHALE)))
}

#[test]
fn figures_in_the_millions_stay_inside_their_tiles() {
    let (mut app, _network) = start(Root::of::<Whale>());
    app.wait_for("$2,268,633.27").unwrap();
    let nodes = app.screen().layout_nodes();
    let bounds = |id: &str| {
        let node = nodes
            .iter()
            .find(|node| node.test_id.as_deref() == Some(id))
            .unwrap_or_else(|| panic!("no view `{id}`"));
        let (x, width) = (node.bounds.origin.x.0, node.bounds.size.width.0);
        (x, x + width)
    };
    for (tile, rows) in [
        (
            "margin-health",
            &["liquidation-buffer", "maintenance-margin", "cross-equity"][..],
        ),
        (
            "margin",
            &["margin-used", "withdrawable", "isolated-margin"][..],
        ),
        ("exposure", &["long", "short", "net-exposure"][..]),
        ("profit-loss", &["funding"][..]),
    ] {
        let (_, edge) = bounds(&format!("account-summary-tile-{tile}"));
        for row in rows {
            let (_, right) = bounds(&format!("account-summary-{row}-value"));
            assert!(right <= edge + 0.5, "{row}: {right} past {edge}");
        }
    }
    // Shown whole, not cut short.
    for whole in [
        "$2,268,633.27",
        "$324,142.28",
        "-$25,931,382.67",
        // Hyperliquid's paid, as received.
        "+$54,321.00",
    ] {
        assert!(app.shows(whole), "{whole}");
    }
}

#[test]
fn an_account_that_holds_nothing_says_so() {
    let (mut app, _network) = start(Root::of::<Idle>());
    app.wait_for("No balances").unwrap();
    app.wait_for("0.045% / 0.015%").unwrap();
    assert!(reads(&app, "equity", "$0.00"));
    assert!(reads(&app, "unrealized-pnl", "$0.00"));
    assert!(reads(&app, "leverage", NONE));
    assert!(reads(&app, "margin-ratio", NONE));
    assert!(reads(&app, "open-positions", "0"));
    assert!(reads(&app, "isolated-positions", "0"));
    assert!(reads(&app, "next-tier", NONE));
    assert!(!app.shows("COST"), "no titles over no rows");
}

/// One watching an address with two accounts on Lighter.
#[component]
fn Trading() -> impl IntoElement {
    workspace_of(WidgetKind::AccountSummary, Some(lighter_account(TRADER)))
}

#[test]
fn a_lighter_address_is_summed_up_from_what_lighter_reports() {
    let (mut app, network) = start(Root::of::<Trading>());
    // (Its history's total is also what its parts come to.)
    for arrived in ["$23,900.00", "+$72.00", "LIT"] {
        app.wait_for(arrived).unwrap();
    }
    for (id, read) in [
        // The headline: its accounts' history together, and its day.
        ("total", "$24,400.00"),
        ("day-change", "+$72.00"),
        // Margin health: its main account's ratio, the nearer liquidation.
        ("margin-ratio", "4.29%"),
        ("liquidation-buffer", "$22,940.00"),
        ("maintenance-margin", "$960.00"),
        ("cross-equity", "$23,900.00"),
        ("equity", "$23,900.00"),
        ("margin-used", "$2,231.00"),
        ("withdrawable", "$21,700.00"),
        ("isolated-positions", "1"),
        ("isolated-margin", "$650.00"),
        // Exposure: the BTC long against the SOL and BTC shorts.
        ("long", "$31,000.00"),
        ("short", "$7,120.00"),
        ("net-exposure", "+$23,880.00"),
        ("position-value", "$38,120.00"),
        ("open-positions", "3"),
        ("largest", "BTC · 81%"),
        ("unrealized-pnl", "+$1,490.00"),
        ("funding", "-$9.75"),
        ("week-pnl", "+$504.00"),
        ("month-pnl", "+$500.00"),
        ("day-volume", "$26,400.00"),
        // What Lighter does not report without a token is a dash.
        ("fee-tier", NONE),
        ("perp-rates", NONE),
        ("staked", NONE),
        ("vault-equity", NONE),
        // The parts: the perpetuals, and its tokens at Lighter's prices.
        ("part-perps", "$23,900.00"),
        ("part-spot", "$500.00"),
    ] {
        assert!(reads(&app, id, read), "{id}: {read}");
    }
    // Its balances: what it holds, not what backs its perpetuals.
    for shown in ["USDC", "250", "100", "90", "$250.00"] {
        assert!(app.shows(shown), "{shown}");
    }
    assert!(!app.shows("No balances") && !app.shows("does not report"));

    // Its accounts once for its margin, its balances and its history, a
    // week of hours and every day of each, and the assets' prices.
    let requests = asked_lighter(&network);
    assert_eq!(requests.len(), 6, "{requests:?}");
    let asked_for = |start: &str| {
        requests
            .iter()
            .filter(|request| request.starts_with(start))
            .count()
    };
    assert_eq!(asked_for("/api/v1/account?"), 1);
    assert_eq!(asked_for("/api/v1/pnl?"), 4);
    assert_eq!(asked_for("/api/v1/assetDetails"), 1);
    assert!(
        asked(&network).is_empty(),
        "nothing is asked of Hyperliquid"
    );
}

/// One watching an account on Hyperliquid and an address on Lighter.
#[component]
fn Everywhere() -> impl IntoElement {
    let saved = accounts(&[on("hyperliquid", USER), on("lighterxyz", TRADER)]);
    workspace_of(WidgetKind::AccountSummary, Some(saved))
}

#[test]
fn accounts_on_several_exchanges_are_one_summary() {
    let (mut app, _network) = start(Root::of::<Everywhere>());
    for arrived in ["$37,009.48", "$37,458.00", "Tier 1", "LIT", "PURR"] {
        app.wait_for(arrived).unwrap();
    }
    for (id, read) in [
        // What both are worth and made, together.
        ("total", "$37,458.00"),
        ("day-change", "+$130.00"),
        ("equity", "$37,009.48"),
        ("cross-equity", "$37,004.51"),
        ("maintenance-margin", "$1,021.00"),
        // The ratio of the one nearer liquidation, not of the sums.
        ("margin-ratio", "4.29%"),
        ("open-positions", "5"),
        ("isolated-positions", "2"),
        ("long", "$31,100.03"),
        ("short", "$13,220.00"),
        ("unrealized-pnl", "+$1,589.99"),
        ("funding", "+$2.75"),
        ("month-pnl", "+$2,950.00"),
        // What one of them alone reports is its own.
        ("fee-tier", "Tier 1"),
        ("staked", "10 HYPE"),
        ("vault-count", "1"),
        // The tokens of each at its own exchange's prices.
        ("part-spot", "$1,514.63"),
    ] {
        assert!(reads(&app, id, read), "{id}: {read}");
    }
    // The strip counts them, and each balance says whose it is.
    assert!(app
        .screen()
        .matches("id:account-summary-account >> text:2 accounts")
        .unwrap());
    assert_eq!(
        app.screen()
            .count("id:account-summary-holdings >> label:Lighter 0xe117…b9a8")
            .unwrap(),
        2
    );
    assert!(
        app.screen()
            .count("id:account-summary-holdings >> label:Hyperliquid 0x0123…4567")
            .unwrap()
            >= 2
    );
}
