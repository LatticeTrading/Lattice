use std::{
    rc::Rc,
    sync::{Arc, Mutex},
};

use nsh_core::{
    MountItem,
    platform::{Idiom, Os},
    prelude::*,
    shader::ShaderLayer,
};
use nsh_engine::{EngineConfig, HttpBody};
use nsh_testing::{Headless, MemoryStorage, StubNetwork, StubRoute, WAIT};
use serde_json::{Value, json};

use super::{
    frame::{Config, Frame, Join, ROW, reach},
    overlay,
    session::Ladder,
    settings::{OrderBookSettings, Toggle},
    shader::{self, ROW_CAPACITY},
};
use crate::{
    data::{
        DataServices,
        ferris::{AmountUnit, BookTraits, FerrisConfig, MarketSelection, OrderBook},
    },
    layout,
    shell::clock::Clock,
    storage::StoredWorkspace,
    widgets::{
        WidgetKind,
        common::books::{
            BookLayout, BookSettings, Grouping, Layer, Logo, Pair, Side, UNAVAILABLE, WAITING,
            testing::{
                CLEAR, FERRIS, GREEN, RED, Record, Ruled, book, colors, drawn, palette, pixel,
                same, side, stub_book,
            },
        },
        picker::Picker,
    },
};

fn bybit() -> MarketSelection {
    MarketSelection::bybit_btc_usdt()
}

fn binance() -> MarketSelection {
    MarketSelection {
        exchange: "binance".into(),
        ..bybit()
    }
}

/// One order a side: a bid at `price`, an ask `spread` above it.
fn quote(price: f64, spread: f64) -> Option<Rc<OrderBook>> {
    book(&[(price, 1.0)], &[(price + spread, 1.0)])
}

fn layer(pair: Pair, market: MarketSelection) -> Layer {
    Layer {
        pair,
        market: Rc::new(market),
        key: pair.id().to_owned(),
    }
}

/// The old tests' configuration: two books at raw prices, a row for every
/// price step, the ladder running down.
fn config() -> Config {
    Config {
        layout: BookLayout::Vertical,
        depth: 200,
        grouping: 0.0,
        all_levels: true,
        only_loaded: false,
        mid_price: true,
        divider: false,
        cumulative: false,
        separate: false,
        layers: vec![
            layer(Pair::Primary, bybit()),
            layer(Pair::Secondary, binance()),
        ],
        warning: None,
        mixed_units: None,
    }
}

/// The primary book alone.
fn single() -> Config {
    Config {
        layers: vec![layer(Pair::Primary, bybit())],
        ..config()
    }
}

fn build(
    config: &Config,
    books: &[Option<Rc<OrderBook>>],
    (width, height): (f64, f64),
    center: Option<f64>,
) -> Frame {
    Frame::build(Rc::new(config.clone()), books, width, height, center, None)
}

fn near(left: f64, right: f64) -> bool {
    (left - right).abs() < 1e-6
}

const LAYOUTS: [BookLayout; 2] = [BookLayout::Vertical, BookLayout::Horizontal];

#[test]
fn settings_save_only_what_differs_and_read_leniently() {
    let saved = |settings: &OrderBookSettings| serde_json::to_value(settings).unwrap();
    let defaults = OrderBookSettings::default();
    assert_eq!(saved(&defaults), json!({}));
    assert_eq!(defaults.market(Pair::Primary), Some(bybit()));
    assert_eq!(defaults.market(Pair::Secondary), None);
    assert_eq!(
        (defaults.depth(), defaults.grouping(), defaults.layout()),
        (200, Grouping::Raw, BookLayout::Vertical)
    );
    for toggle in Toggle::ALL {
        assert_eq!(defaults.shows(toggle), toggle == Toggle::MidPrice);
    }
    assert_eq!(defaults.center(), Some(Pair::Primary));

    let settings = defaults
        .clone()
        .with_market(Pair::Secondary, Some(binance()))
        .with_depth(50)
        .with_grouping(Grouping::Ten)
        .with_layout(BookLayout::Horizontal)
        .with_toggle(Toggle::MidPrice, false)
        .with_toggle(Toggle::Cumulative, true)
        .with_center(Some(Pair::Secondary));
    let value = saved(&settings);
    assert_eq!(value["depth"], 50);
    assert_eq!(value["priceGrouping"], 10);
    assert_eq!(value["layout"], "horizontal");
    assert_eq!(value["showMidPriceLine"], false);
    assert_eq!(value["showCumulativeAmount"], true);
    assert_eq!(value["centerTarget"], "secondary");
    assert_eq!(value["secondaryMarket"]["exchange"], "binance");
    assert_eq!(value.get("primaryMarket"), None);
    assert_eq!(value.get("showAllLevels"), None);
    assert_eq!(OrderBookSettings::from(value), settings);
    let tenths = defaults.clone().with_grouping(Grouping::Tenth);
    assert_eq!(saved(&tenths)["priceGrouping"], 0.1);

    // The primary cleared on purpose is saved as such.
    let cleared = defaults.clone().with_market(Pair::Primary, None);
    assert_eq!(saved(&cleared), json!({ "primaryMarket": null }));
    assert_eq!(
        OrderBookSettings::from(saved(&cleared)).market(Pair::Primary),
        None
    );

    // What the old app read, and what it could not.
    let read = OrderBookSettings::from;
    assert_eq!(read(json!({ "depth": 1200.9 })).depth(), 1_000);
    assert_eq!(read(json!({ "depth": 5 })).depth(), 10);
    assert_eq!(read(json!({ "depth": "250" })).depth(), 250);
    assert_eq!(read(json!({ "depth": "deep" })).depth(), 200);
    assert_eq!(
        read(json!({ "priceGrouping": "10" })).grouping(),
        Grouping::Ten
    );
    assert_eq!(
        read(json!({ "priceGrouping": 0.02 })).grouping(),
        Grouping::Raw
    );
    assert!(!read(json!({ "showAllLevels": "true" })).shows(Toggle::AllLevels));
    assert!(read(json!({ "showMidPriceLine": "no" })).shows(Toggle::MidPrice));
    let damaged = read(json!({ "primaryMarket": "BTC", "layout": "diagonal" }));
    assert_eq!(damaged, OrderBookSettings::default());
    assert_eq!(read(json!([1, 2])), OrderBookSettings::default());
}

#[test]
fn the_book_followed_is_one_that_is_drawn() {
    let read = OrderBookSettings::from;
    let second = serde_json::to_value(binance()).unwrap();
    let paired = read(json!({ "centerTarget": "secondary", "secondaryMarket": second }));
    assert_eq!(paired.center(), Some(Pair::Secondary));
    assert_eq!(
        read(json!({ "centerTarget": "primary" })).center(),
        Some(Pair::Primary)
    );
    // None, by choice or because the old app did not know the name.
    for none in [json!(null), json!("tertiary"), json!(true)] {
        assert_eq!(read(json!({ "centerTarget": none })).center(), None);
    }
    // A book that is gone is not followed.
    assert_eq!(read(json!({ "primaryMarket": null })).center(), None);
    assert_eq!(read(json!({ "centerTarget": "secondary" })).center(), None);

    // The second pair saved with more metadata streams the primary's book:
    // it is not drawn, so not followed either.
    let again = MarketSelection {
        market_id: Some("BTCUSDT".into()),
        ..bybit()
    };
    let repeated = OrderBookSettings::default()
        .with_market(Pair::Secondary, Some(again))
        .with_center(Some(Pair::Secondary));
    assert!(repeated.duplicate());
    assert_eq!(repeated.center(), None);
    assert_eq!(repeated.layers().len(), 1);
    assert_eq!(
        repeated.warning(),
        Some("Pair 2 duplicates Primary and is not rendered.")
    );

    assert_eq!(repeated.mixed_units(), None);

    // Two books have something to say should their units differ, grouped
    // or not; their books decide whether it is said.
    let pairs = |settings: &OrderBookSettings| -> Vec<Pair> {
        settings.layers().iter().map(|layer| layer.pair).collect()
    };
    let two = OrderBookSettings::default().with_market(Pair::Secondary, Some(binance()));
    assert_eq!(pairs(&two), [Pair::Primary, Pair::Secondary]);
    assert_eq!(two.warning(), None);
    let combined = Some("Pair quantities are in different units and are combined as sent.");
    assert_eq!(two.mixed_units(), combined);
    assert_eq!(
        two.clone().with_grouping(Grouping::One).mixed_units(),
        combined
    );
    assert_eq!(
        two.clone()
            .with_toggle(Toggle::Separate, true)
            .mixed_units(),
        Some("Pair quantities are in different units.")
    );
    assert_eq!(OrderBookSettings::default().mixed_units(), None);
    // The second pair alone is drawn alone.
    assert_eq!(
        pairs(&two.with_market(Pair::Primary, None)),
        [Pair::Secondary]
    );
}

#[test]
fn two_books_in_one_base_asset_are_combined_without_a_warning() {
    let said = |unit: AmountUnit, currency: &str| {
        let mut book = (*quote(100.0, 1.0).unwrap()).clone();
        book.traits = BookTraits {
            unit: Some(unit),
            currency: Some(currency.into()),
            liquidity: None,
        };
        Some(Rc::new(book))
    };
    let mixed = Some("Pair quantities are in different units and are combined as sent.");
    let config = Config {
        mixed_units: mixed,
        ..config()
    };
    let warned =
        |books: &[Option<Rc<OrderBook>>]| build(&config, books, (800.0, 600.0), None).warning;
    // Bybit's and KuCoin's BTC/USDT, both in BTC: nothing to say, and the
    // plot starts at the top.
    let btc = [said(AmountUnit::Base, "BTC"), said(AmountUnit::Base, "BTC")];
    assert_eq!(warned(&btc), None);
    assert_eq!(build(&config, &btc, (800.0, 600.0), None).plot_top, 0.0);
    // Contracts, another base, or a server that says nothing.
    for other in [
        said(AmountUnit::Contracts, "BTC"),
        said(AmountUnit::Base, "ETH"),
        quote(100.0, 1.0),
    ] {
        assert_eq!(warned(&[said(AmountUnit::Base, "BTC"), other]), mixed);
    }
    // Nothing is said of a book that has not arrived.
    assert_eq!(warned(&[said(AmountUnit::Contracts, "BTC"), None]), None);
}

#[test]
fn two_books_scale_apart_on_one_ladder() {
    let books = [
        book(&[(100.0, 2.0), (99.0, 3.0)], &[(101.0, 4.0), (102.0, 1.0)]),
        book(
            &[(98.0, 20.0), (97.0, 30.0)],
            &[(103.0, 40.0), (104.0, 10.0)],
        ),
    ];
    let config = Config {
        mixed_units: Some("Pair quantities are in different units."),
        ..config()
    };
    let frame = build(&config, &books, (800.0, 600.0), None);
    // Books that do not say what their quantities are in are warned of;
    // the plot is right of the price column and below the warning.
    assert_eq!(frame.warning, config.mixed_units);
    assert_eq!(
        (frame.plot_left, frame.plot_top, frame.baseline),
        (66.0, 20.0, 66.0)
    );
    // Book by book: bids from the best down, then asks from the best up.
    let order: Vec<_> = frame
        .levels
        .iter()
        .map(|level| (level.layer, level.side, level.price))
        .collect();
    assert_eq!(
        order,
        [
            (0, Side::Bid, 100.0),
            (0, Side::Bid, 99.0),
            (0, Side::Ask, 101.0),
            (0, Side::Ask, 102.0),
            (1, Side::Bid, 98.0),
            (1, Side::Bid, 97.0),
            (1, Side::Ask, 103.0),
            (1, Side::Ask, 104.0),
        ]
    );
    // Each bar reaches as far as its running amount over its own book's
    // deepest, from the plot's left edge.
    let reaches: Vec<_> = frame.levels.iter().map(|level| level.reach).collect();
    assert_eq!(reaches, [0.4, 1.0, 0.8, 1.0, 0.4, 1.0, 0.8, 1.0]);
    for level in &frame.levels {
        assert_eq!(level.rect[0], 66.0);
        assert!(near(level.rect[2], 66.0 + level.reach * 734.0));
    }
    // The ladder is centered on the first book's mid price, a step a row.
    assert_eq!((frame.center_price, frame.step), (100.5, 1.0));
    assert_eq!(frame.mids, [Some(100.5), Some(100.5)]);
    assert!(near(frame.axis_of(100.5), 310.0));
    assert!(near(frame.axis_of(101.0) - frame.axis_of(102.0), ROW));
}

#[test]
fn the_last_level_loaded_runs_on_to_the_plots_edge() {
    let books = [
        book(
            &[(85_900.0, 1.0), (85_800.0, 2.0), (85_700.0, 100.0)],
            &[(86_100.0, 2.0), (86_200.0, 4.0), (86_300.0, 100.0)],
        ),
        book(
            &[(85_900.0, 3.0), (85_800.0, 6.0), (85_700.0, 100.0)],
            &[(86_100.0, 1.0), (86_200.0, 2.0), (86_300.0, 100.0)],
        ),
    ];
    for layout in LAYOUTS {
        let vertical = layout == BookLayout::Vertical;
        for all_levels in [false, true] {
            let config = Config {
                layout,
                all_levels,
                depth: 2,
                grouping: 100.0,
                ..config()
            };
            // Along the ladder: a bar's two edges.
            let along = |rect: [f64; 4]| {
                if vertical {
                    (rect[1], rect[3])
                } else {
                    (rect[0], rect[2])
                }
            };
            let middle = if vertical { 300.0 } else { 400.0 };
            // Panned far above, far above again, and far below the book.
            for center in [120_000.0, 600_000.0, 10_000.0] {
                let frame = build(&config, &books, (800.0, 600.0), Some(center));
                assert!((frame.axis_of(center) - middle).abs() <= ROW / 2.0);
                assert_eq!(frame.levels.len(), 8);
                for price in [10_000.0, 85_800.0, 86_000.0, 86_200.0, 600_000.0] {
                    let back = frame.price_at(frame.axis_of(price));
                    assert!((back - price).abs() < 1e-4, "{price} came back {back}");
                }
                // The side toward the book: its last level of each book
                // runs through the plot's middle, as deep as that book got.
                let asks = center >= 86_000.0;
                for (book, deepest) in [(0, 6.0), (1, 9.0)] {
                    let level = &frame.levels[book * 4 + usize::from(asks) * 2 + 1];
                    let (from, to) = along(level.rect);
                    assert!(from <= middle && to >= middle, "{layout:?} {center}");
                    assert!(level.open);
                    let across = if vertical {
                        level.rect[2] - frame.plot_left
                    } else {
                        frame.plot_bottom - level.rect[1]
                    };
                    let length = if vertical { 734.0 } else { 600.0 };
                    assert!(near(across, level.cumulative / deepest * length));
                }
                // Stopped at the loaded levels, nothing reaches it.
                let stopped = Config {
                    only_loaded: true,
                    ..config.clone()
                };
                let frame = build(&stopped, &books, (800.0, 600.0), Some(center));
                for level in &frame.levels {
                    let (from, to) = along(level.rect);
                    assert!(!(from <= middle && to >= middle));
                    assert!(!level.open);
                }
            }
            // Asks never run on downward.
            let one_sided = [book(&[], &[(86_100.0, 2.0)]), None];
            let frame = build(&config, &one_sided, (800.0, 600.0), Some(10_000.0));
            assert_eq!(frame.levels.len(), 1);
            let (from, to) = along(frame.levels[0].rect);
            assert!(!(from <= middle && to >= middle));
        }
    }
}

#[test]
fn prices_that_hold_orders_take_a_row_each_unless_every_step_does() {
    let books = [book(
        &[(100.0, 2.0), (99.4, 3.0)],
        &[(100.1, 4.0), (100.7, 5.0)],
    )];
    for layout in LAYOUTS {
        for grouping in [0.0, 0.1] {
            let config = Config {
                layout,
                grouping,
                all_levels: false,
                ..single()
            };
            let compact = build(&config, &books, (320.0, 180.0), None);
            assert_eq!(compact.occupied.len(), 4);
            for (held, price) in compact.occupied.iter().zip([99.4, 100.0, 100.1, 100.7]) {
                assert!((held - price).abs() < 1e-8);
            }
            // A row apart, whatever is between them.
            let axis = |price: f64| compact.axis_of(price);
            assert!(near((axis(100.1) - axis(100.7)).abs(), ROW));
            assert!(near((axis(100.0) - axis(99.4)).abs(), ROW));
            assert!(near((axis(100.0) - axis(100.1)).abs(), ROW));
            // The mid price is at the plot's middle, between two rows.
            let middle = if layout == BookLayout::Vertical {
                90.0
            } else {
                160.0
            };
            assert!(near(axis(100.05), middle));
            // A row's pan moves every price a row.
            let panned = build(
                &config,
                &books,
                (320.0, 180.0),
                Some(compact.shifted_center(1.0)),
            );
            assert!(near((panned.axis_of(100.7) - axis(100.7)).abs(), ROW));

            let stepped = Config {
                all_levels: true,
                ..config.clone()
            };
            let stepped = build(&stepped, &books, (320.0, 180.0), None);
            assert!(stepped.occupied.is_empty());
            assert!(near(stepped.step, 0.1));
            let apart = (stepped.axis_of(100.7) - stepped.axis_of(100.1)).abs();
            assert!(near(apart, 6.0 * ROW));
        }
    }
}

#[test]
fn rows_are_shared_by_the_books_and_levels_without_an_amount_have_none() {
    let primary = Some(Rc::new(OrderBook {
        bids: side(&[
            (100.0, 2.0),
            (99.9, 0.0),
            (99.8, f64::NAN),
            (99.7, -1.0),
            (99.0, 3.0),
        ]),
        asks: side(&[(100.1, 1.0), (100.7, 2.0)]),
        timestamp: 1.0,
        traits: BookTraits::default(),
    }));
    let books = [
        primary,
        book(&[(100.0, 4.0), (99.9, 1.0)], &[(100.4, 2.0), (100.7, 3.0)]),
    ];
    let config = Config {
        all_levels: false,
        ..config()
    };
    let frame = build(&config, &books, (320.0, 240.0), None);
    assert_eq!(frame.occupied, [99.0, 99.9, 100.0, 100.1, 100.4, 100.7]);
    let amounts: Vec<_> = frame.levels.iter().map(|level| level.amount).collect();
    assert_eq!(amounts, [2.0, 3.0, 1.0, 2.0, 4.0, 1.0, 2.0, 3.0]);
    // A running amount that is no number is counted again.
    assert_eq!(frame.levels[1].cumulative, 5.0);
    // Both books' levels at one price are in one row.
    assert_eq!(frame.levels[0].rect[1], frame.levels[4].rect[1]);
    assert_eq!(frame.levels[3].rect[1], frame.levels[7].rect[1]);

    let smaller = build(
        &config,
        &[book(&[], &[(101.0, 1.0)]), None],
        (320.0, 240.0),
        None,
    );
    assert_eq!(smaller.occupied, [101.0]);
    assert!((smaller.axis_of(101.0) - 120.0).abs() <= ROW / 2.0);
    let empty = build(
        &config,
        &[book(&[(100.0, 0.0)], &[]), None],
        (320.0, 240.0),
        None,
    );
    assert!(empty.levels.is_empty() && empty.occupied.is_empty());
    assert!(!empty.has_prices);
}

#[test]
fn groups_take_prices_outward_and_sum_what_they_hold() {
    let books = [book(
        &[(100.09, 2.0), (100.01, 3.0), (99.91, 50.0)],
        &[(100.11, 4.0), (100.19, 1.0), (100.21, 50.0)],
    )];
    let config = Config {
        grouping: 0.1,
        depth: 2,
        ..single()
    };
    // The best two levels a side, then grouped: bids down, asks up.
    let grouped = build(&config, &books, (300.0, 200.0), None);
    let prices: Vec<_> = grouped.levels.iter().map(|level| level.price).collect();
    assert_eq!(prices, [100.0, 100.2]);
    let amounts: Vec<_> = grouped.levels.iter().map(|level| level.amount).collect();
    assert_eq!(amounts, [5.0, 5.0]);
    assert!(near(grouped.levels[0].total, 500.21));
    assert!(near(grouped.levels[1].total, 500.63));
    assert_eq!(grouped.levels[1].cumulative, 5.0);
    assert_eq!(grouped.step, 0.1);

    let raw = Config {
        grouping: 0.0,
        ..config
    };
    let raw = build(&raw, &books, (300.0, 200.0), None);
    let prices: Vec<_> = raw.levels.iter().map(|level| level.price).collect();
    assert_eq!(prices, [100.09, 100.01, 100.11, 100.19]);
    // The largest step every raw price is whole steps from the lowest by.
    assert!(near(raw.step, 0.02));

    // A group's row is a row whatever lies between two groups.
    let wide = [book(
        &[(100.0, 1.0), (99.8, 2.0)],
        &[(100.2, 1.0), (100.5, 2.0)],
    )];
    let frame = build(
        &Config {
            grouping: 0.1,
            ..single()
        },
        &wide,
        (320.0, 180.0),
        None,
    );
    assert!(near(
        (frame.axis_of(100.0) - frame.axis_of(99.8)).abs(),
        2.0 * ROW
    ));
}

#[test]
fn odd_books_still_make_a_frame() {
    let odd = [
        book(&[], &[]),
        book(&[(f64::NAN, 1.0)], &[(f64::INFINITY, 1.0)]),
        book(&[(100.0, 1.0)], &[(100.0, 1.0)]),
        book(&[(102.0, 1.0)], &[(101.0, 1.0)]),
        book(&[(100.0, 1.0)], &[(101.0, 1.0)]),
        None,
    ];
    for layout in LAYOUTS {
        for only in &odd {
            let config = Config { layout, ..single() };
            let frame = build(&config, std::slice::from_ref(only), (320.0, 180.0), None);
            for level in &frame.levels {
                assert!(level.rect.iter().all(|edge| edge.is_finite()));
            }
            assert!(frame.center_price.is_finite() && frame.steps());
            assert!(
                frame
                    .tiles
                    .iter()
                    .all(|tile| tile.rows.len() <= ROW_CAPACITY)
            );
        }
    }
    // A widget with no size yet.
    let frame = build(&single(), &[quote(100.0, 1.0)], (f64::NAN, 0.0), None);
    assert_eq!((frame.width, frame.height), (1.0, 1.0));
}

/// The primary book's quotes move; the second pair's follow `second`.
fn follow(
    ladder: &mut Ladder,
    config: &Config,
    target: Option<Pair>,
    books: &[Option<Rc<OrderBook>>],
) -> Frame {
    ladder.frame(&Rc::new(config.clone()), target, books, 640.0, 480.0)
}

#[test]
fn the_ladder_follows_the_chosen_books_mid_price() {
    for layout in LAYOUTS {
        for all_levels in [false, true] {
            let config = Config {
                layout,
                all_levels,
                grouping: 1.0,
                ..config()
            };
            let mut ladder = Ladder::default();
            let primary = Some(Pair::Primary);
            let frame = follow(
                &mut ladder,
                &config,
                primary,
                &[quote(100.0, 1.0), quote(200.0, 1.0)],
            );
            assert_eq!(frame.center_price, 100.5);
            // The other book moving moves nothing.
            let frame = follow(
                &mut ladder,
                &config,
                primary,
                &[quote(100.0, 1.0), quote(210.0, 1.0)],
            );
            assert_eq!(frame.center_price, 100.5);
            let frame = follow(
                &mut ladder,
                &config,
                primary,
                &[quote(110.0, 1.0), quote(210.0, 1.0)],
            );
            assert_eq!(frame.center_price, 110.5);
            let middle = if layout == BookLayout::Vertical {
                (frame.plot_top + frame.plot_bottom) / 2.0
            } else {
                (frame.plot_left + frame.plot_right) / 2.0
            };
            assert_eq!(frame.axis_of(110.5), middle);

            let second = Some(Pair::Secondary);
            let frame = follow(
                &mut ladder,
                &config,
                second,
                &[quote(110.0, 1.0), quote(210.0, 1.0)],
            );
            assert_eq!(frame.center_price, 210.5);
            let frame = follow(
                &mut ladder,
                &config,
                second,
                &[quote(110.0, 1.0), quote(220.0, 1.0)],
            );
            assert_eq!(frame.center_price, 220.5);
            // Following none holds the view where it was.
            let frame = follow(
                &mut ladder,
                &config,
                None,
                &[quote(250.0, 1.0), quote(350.0, 1.0)],
            );
            assert_eq!(frame.center_price, 220.5);
            let frame = follow(
                &mut ladder,
                &config,
                primary,
                &[quote(250.0, 1.0), quote(350.0, 1.0)],
            );
            assert_eq!(frame.center_price, 250.5);
        }
    }
}

#[test]
fn rows_stay_put_while_the_price_moves_inside_one_and_a_pan_moves_a_whole_row() {
    for all_levels in [false, true] {
        let config = Config {
            all_levels,
            grouping: 1.0,
            ..single()
        };
        let mut ladder = Ladder::default();
        let mut followed = 0.0;
        for mid in [82_475.6, 82_475.9] {
            let books = [quote(mid - 0.1, 0.2)];
            let frame = follow(&mut ladder, &config, Some(Pair::Primary), &books);
            assert!(near(frame.center_price, mid));
            // The ask's row is the one above the plot's middle, both times.
            followed = frame.axis_of(82_476.0);
            assert_eq!(followed, (frame.plot_top + frame.plot_bottom - ROW) / 2.0);
        }
        // A notch down: a row toward lower prices, and no more following.
        let books = [quote(82_475.8, 0.2)];
        let frame = follow(&mut ladder, &config, Some(Pair::Primary), &books);
        ladder.pan(&frame, -1.0);
        let panned = follow(&mut ladder, &config, None, &books);
        assert_eq!(panned.axis_of(82_476.0), followed - ROW);
        let frame = follow(&mut ladder, &config, None, &[quote(82_480.0, 1.0)]);
        assert_eq!(frame.center_price, panned.center_price);
    }
}

#[test]
fn a_pan_holds_until_the_ladder_is_laid_out_anew_or_followed_again() {
    for all_levels in [false, true] {
        let config = Config {
            all_levels,
            grouping: 1.0,
            ..config()
        };
        let mut ladder = Ladder::default();
        let second = Some(Pair::Secondary);
        let books = [quote(100.0, 1.0), quote(200.0, 1.0)];
        let frame = follow(&mut ladder, &config, second, &books);
        assert_eq!(frame.center_price, 200.5);
        ladder.pan(&frame, -1.0);
        // A row down the ladder: the price step below, or the row between
        // the two books' prices.
        let manual = if all_levels { 199.5 } else { 150.5 };
        let frame = follow(
            &mut ladder,
            &config,
            None,
            &[quote(100.0, 1.0), quote(300.0, 1.0)],
        );
        assert_eq!(frame.center_price, manual);
        let frame = follow(
            &mut ladder,
            &config,
            None,
            &[quote(400.0, 1.0), quote(500.0, 1.0)],
        );
        assert_eq!(frame.center_price, manual);
        let frame = follow(
            &mut ladder,
            &config,
            second,
            &[quote(400.0, 1.0), quote(500.0, 1.0)],
        );
        assert_eq!(frame.center_price, 500.5);

        // Asked to start over, it fits the first book again.
        ladder.reset();
        let frame = follow(
            &mut ladder,
            &config,
            None,
            &[quote(400.0, 1.0), quote(500.0, 1.0)],
        );
        assert_eq!(frame.center_price, 400.5);
    }
    // What lays the ladder out anew, and what does not.
    let base = config();
    assert!(base.relaid_by(&Config {
        layout: BookLayout::Horizontal,
        ..config()
    }));
    assert!(base.relaid_by(&Config {
        grouping: 1.0,
        ..config()
    }));
    assert!(base.relaid_by(&Config {
        all_levels: false,
        ..config()
    }));
    assert!(base.relaid_by(&single()));
    assert!(!base.relaid_by(&Config {
        depth: 50,
        divider: true,
        cumulative: true,
        ..config()
    }));
}

#[test]
fn a_followed_book_that_is_waiting_or_one_sided_holds_the_view() {
    let config = config();
    let mut ladder = Ladder::default();
    let second = Some(Pair::Secondary);
    // Never the other book's price.
    let frame = follow(&mut ladder, &config, second, &[quote(100.0, 1.0), None]);
    assert_eq!(frame.center_price, 0.0);
    let frame = follow(
        &mut ladder,
        &config,
        second,
        &[quote(100.0, 1.0), quote(200.0, 1.0)],
    );
    assert_eq!(frame.center_price, 200.5);
    let bids_only = book(&[(300.0, 1.0)], &[]);
    let frame = follow(
        &mut ladder,
        &config,
        second,
        &[quote(400.0, 1.0), bids_only],
    );
    assert_eq!(frame.center_price, 200.5);
    let frame = follow(
        &mut ladder,
        &config,
        second,
        &[quote(400.0, 1.0), quote(500.0, 1.0)],
    );
    assert_eq!(frame.center_price, 500.5);

    // With none followed, the first prices fix the view.
    let mut ladder = Ladder::default();
    let frame = follow(&mut ladder, &single(), None, &[quote(100.0, 1.0)]);
    assert_eq!(frame.center_price, 100.5);
    let frame = follow(&mut ladder, &single(), None, &[quote(200.0, 1.0)]);
    assert_eq!(frame.center_price, 100.5);
    let frame = follow(
        &mut ladder,
        &single(),
        Some(Pair::Primary),
        &[quote(200.0, 1.0)],
    );
    assert_eq!(frame.center_price, 200.5);
}

/// One book on a plot of four rows, 100 points across: asks at 102 and
/// 101 above bids at 100 and 99, reaching a whole, a half, a quarter and
/// a half of the way.
fn four_rows(layout: BookLayout, only_loaded: bool) -> Frame {
    let config = Config {
        layout,
        all_levels: false,
        only_loaded,
        ..single()
    };
    let books = [book(
        &[(100.0, 1.0), (99.0, 1.0)],
        &[(101.0, 2.0), (102.0, 2.0)],
    )];
    let size = match layout {
        BookLayout::Vertical => (166.0, 88.0),
        BookLayout::Horizontal => (88.0, 100.0),
    };
    build(&config, &books, size, None)
}

#[test]
fn rows_carry_each_series_reach_and_how_it_joins() {
    let stopped = four_rows(BookLayout::Vertical, true);
    let [tile] = stopped.tiles.as_slice() else {
        panic!("88 points are one tile: {}", stopped.tiles.len());
    };
    // The plot's four rows, one before them and one after.
    assert_eq!((tile.offset, tile.length, tile.start), (0.0, 88.0, -22.0));
    let none = [0.0; 4];
    assert_eq!(
        tile.rows,
        [
            none,
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.5, 0.0, 0.0],
            [0.25, 0.0, 0.0, 0.0],
            [0.5, 0.0, 0.0, 0.0],
            none,
        ]
    );
    // Left open, each side's last level joins the rows beyond it, which
    // it fills.
    let open = four_rows(BookLayout::Vertical, false);
    assert_eq!(
        open.tiles[0].rows,
        [
            [0.0, reach(1.0, Join::Through), 0.0, 0.0],
            [0.0, reach(1.0, Join::Outward), 0.0, 0.0],
            [0.0, 0.5, 0.0, 0.0],
            [0.25, 0.0, 0.0, 0.0],
            [reach(0.5, Join::Outward), 0.0, 0.0, 0.0],
            [reach(0.5, Join::Through), 0.0, 0.0, 0.0],
        ]
    );
    // Across the pane prices rise to the right: bids first, running on to
    // the left.
    let across = four_rows(BookLayout::Horizontal, false);
    assert_eq!(
        across.tiles[0].rows,
        [
            [reach(0.5, Join::Through), 0.0, 0.0, 0.0],
            [reach(0.5, Join::Outward), 0.0, 0.0, 0.0],
            [0.25, 0.0, 0.0, 0.0],
            [0.0, 0.5, 0.0, 0.0],
            [0.0, reach(1.0, Join::Outward), 0.0, 0.0],
            [0.0, reach(1.0, Join::Through), 0.0, 0.0],
        ]
    );

    // Looking far past the book, every row is beyond one side's last
    // level: above the asks, below the bids.
    let packed = Config {
        all_levels: false,
        ..single()
    };
    let books = [book(
        &[(100.0, 1.0), (99.0, 1.0)],
        &[(101.0, 2.0), (102.0, 2.0)],
    )];
    for (center, beyond) in [
        (200.0, [0.0, reach(1.0, Join::Through), 0.0, 0.0]),
        (10.0, [reach(0.5, Join::Through), 0.0, 0.0, 0.0]),
    ] {
        let far = build(&packed, &books, (166.0, 88.0), Some(center));
        assert!(far.tiles[0].rows.iter().all(|row| *row == beyond));
    }

    // A second book's series are the last two; a plot longer than a view
    // holds is cut, each tile with its own rows.
    let books = [quote(100.0, 1.0), quote(100.0, 1.0)];
    let config = Config {
        only_loaded: true,
        ..config()
    };
    let tall = build(&config, &books, (200.0, 12_000.0), None);
    assert_eq!(tall.tiles.len(), 3);
    let mut end = 0.0;
    for tile in &tall.tiles {
        assert_eq!(tile.offset, end);
        assert!(tile.rows.len() <= ROW_CAPACITY);
        end += tile.length;
    }
    assert_eq!(end, 12_000.0);
    let drawn: Vec<_> = tall
        .tiles
        .iter()
        .flat_map(|tile| &tile.rows)
        .filter(|row| **row != none)
        .collect();
    assert_eq!(drawn, [&[0.0, 1.0, 0.0, 1.0], &[1.0, 0.0, 1.0, 0.0]]);
}

/// The shader's layer for the one tile of `frame`.
fn shader_layer(frame: &Frame) -> ShaderLayer {
    let program = shader::program().expect("the order book's shader compiles");
    let uniforms = shader::uniforms(frame, &frame.tiles[0], &palette());
    let mut warnings = Vec::new();
    let layer = ShaderLayer::new(program, &[("d".into(), uniforms)], &mut warnings);
    assert_eq!(warnings, Vec::<String>::new());
    layer
}

#[test]
fn the_shader_compiles_and_takes_a_tile_in_one_array() {
    let program = shader::program().expect("the order book's shader compiles");
    assert_eq!(program.uniform_size(), 4_096);
    assert!(
        !program.animated(),
        "it draws only when its uniforms change"
    );
    assert!(program.inputs().is_empty());

    let frame = four_rows(BookLayout::Vertical, true);
    let layer = shader_layer(&frame);
    let word = |at: usize| f32::from_bits(layer.uniforms[at]);
    let vector = |at: usize| {
        [
            word(at * 4),
            word(at * 4 + 1),
            word(at * 4 + 2),
            word(at * 4 + 3),
        ]
    };
    // The view: the first row starts a row before the tile, six rows,
    // prices down the view.
    assert_eq!(vector(0), [-22.0, 6.0, 0.0, 0.0]);
    // The primary's bids and asks; no second pair.
    assert_eq!(vector(1), [0.0, 1.0, 0.0, 1.0]);
    assert_eq!(vector(2), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(vector(3), [0.0; 4]);
    // The first row that shows: the asks' at 102, all the way across.
    assert_eq!(vector(6), [0.0, 1.0, 0.0, 0.0]);

    let across = shader_layer(&four_rows(BookLayout::Horizontal, true));
    assert_eq!(f32::from_bits(across.uniforms[2]), 1.0);
    // The second pair alone draws in its own colors.
    let config = Config {
        layers: vec![layer_of_second()],
        ..single()
    };
    let alone = shader_layer(&build(&config, &[quote(100.0, 1.0)], (166.0, 88.0), None));
    let word = |at: usize| f32::from_bits(alone.uniforms[at]);
    assert_eq!([word(4), word(5), word(6), word(7)], [0.0, 0.0, 1.0, 1.0]);
}

fn layer_of_second() -> Layer {
    layer(Pair::Secondary, binance())
}

#[test]
fn the_gpu_draws_bars_where_the_frame_puts_them() {
    let frame = four_rows(BookLayout::Vertical, true);
    let Some(picture) = drawn(shader_layer(&frame), 100.0, 88.0, 1.0) else {
        eprintln!("no GPU here: the order book's pixels were not checked");
        return;
    };
    assert_eq!((picture.width, picture.height), (100, 88));
    // Each row is 22 pixels: one clear, then its bar. The asks at 102
    // reach all the way across, those at 101 half way.
    assert_eq!(pixel(&picture, 0, 0), CLEAR);
    assert_eq!(pixel(&picture, 0, 1), RED);
    assert_eq!(pixel(&picture, 99, 21), RED);
    assert_eq!(pixel(&picture, 50, 22), CLEAR);
    assert_eq!(pixel(&picture, 49, 23), RED);
    assert_eq!(pixel(&picture, 50, 30), CLEAR);
    assert_eq!(pixel(&picture, 49, 43), RED);
    // The bids at 100 a quarter of the way, those at 99 half way.
    assert_eq!(pixel(&picture, 10, 44), CLEAR);
    assert_eq!(pixel(&picture, 24, 45), GREEN);
    assert_eq!(pixel(&picture, 25, 55), CLEAR);
    assert_eq!(pixel(&picture, 0, 66), CLEAR);
    assert_eq!(pixel(&picture, 49, 67), GREEN);
    assert_eq!(pixel(&picture, 50, 80), CLEAR);
    assert_eq!(pixel(&picture, 49, 87), GREEN);

    // Left open, the highest asks join the rows beyond them (the pixel
    // above their bar is theirs), and nothing joins the row toward the
    // spread.
    let open = drawn(
        shader_layer(&four_rows(BookLayout::Vertical, false)),
        100.0,
        88.0,
        1.0,
    )
    .expect("the GPU answered before");
    assert_eq!(pixel(&open, 99, 0), RED);
    assert_eq!(pixel(&open, 10, 22), CLEAR);
    assert_eq!(pixel(&open, 10, 66), CLEAR);

    // At two pixels a point the gap is two pixels, about the rows' edge,
    // and bars are twice as long.
    let dense = drawn(shader_layer(&frame), 100.0, 88.0, 2.0).expect("the GPU answered before");
    assert_eq!((dense.width, dense.height), (200, 176));
    assert_eq!(pixel(&dense, 0, 0), CLEAR);
    assert_eq!(pixel(&dense, 199, 1), RED);
    assert_eq!(pixel(&dense, 199, 42), RED);
    assert_eq!(pixel(&dense, 0, 43), CLEAR);
    assert_eq!(pixel(&dense, 0, 44), CLEAR);
    assert_eq!(pixel(&dense, 99, 45), RED);
    assert_eq!(pixel(&dense, 100, 45), CLEAR);
    assert_eq!(pixel(&dense, 49, 89), GREEN);
    assert_eq!(pixel(&dense, 50, 89), CLEAR);

    // Across the pane bars grow up from the bottom: bids left, asks right.
    let across = drawn(
        shader_layer(&four_rows(BookLayout::Horizontal, true)),
        88.0,
        100.0,
        1.0,
    )
    .expect("the GPU answered before");
    assert_eq!(pixel(&across, 10, 51), GREEN);
    assert_eq!(pixel(&across, 10, 49), CLEAR);
    assert_eq!(pixel(&across, 30, 76), GREEN);
    assert_eq!(pixel(&across, 30, 74), CLEAR);
    assert_eq!(pixel(&across, 22, 99), CLEAR);
    assert_eq!(pixel(&across, 50, 51), RED);
    assert_eq!(pixel(&across, 70, 1), RED);
    assert_eq!(pixel(&across, 87, 0), RED);
}

/// The overlay of `frame`, with the pointer where `pointer` says.
fn overlaid(frame: &Frame, pointer: Option<(f64, f64)>) -> (Record, Vec<Logo>) {
    let drawn = overlay::draw(frame, pointer, &colors(), false);
    (Record::of(&drawn.painter.finish()), drawn.logos)
}

/// The old overlay tests' configuration: the primary book alone, a row
/// per price that holds orders, on 300 by 180 points.
fn compact() -> Config {
    Config {
        all_levels: false,
        ..single()
    }
}

const SMALL: (f64, f64) = (300.0, 180.0);

#[test]
fn the_overlay_writes_prices_totals_and_running_amounts() {
    let books = [book(&[(100.0, 2.0)], &[(102.0, 3.0)])];
    for layout in LAYOUTS {
        let config = Config {
            layout,
            ..compact()
        };
        let (plain, logos) = overlaid(&build(&config, &books, SMALL, None), None);
        assert!(plain.words().contains(&"100.000"));
        assert!(plain.words().contains(&"200"));
        assert_eq!(plain.count("2 BTC"), 0);
        assert!(logos.is_empty());
        if layout == BookLayout::Vertical {
            // The price in its column, the total at the plot's left edge.
            assert_eq!(plain.find("100.000").x, 6.0);
            assert_eq!(plain.find("200").x, 74.0);
            assert_eq!(plain.find("200").y, plain.find("100.000").y);
        } else {
            // Turned on their side, from the plot's bottom at their row.
            let (price, total) = (plain.find("100.000"), plain.find("200"));
            assert_eq!(price.origin, total.origin);
            assert_eq!((price.x, price.origin.1), (0.0, 172.0));
            assert_eq!(total.x, 64.0);
        }
        // The mid price's line and its price.
        assert_eq!(plain.count("101.000"), 1);
        assert_eq!(plain.ruled(colors().accent).len(), 1);

        let config = Config {
            cumulative: true,
            ..config
        };
        let (with, _) = overlaid(&build(&config, &books, SMALL, None), None);
        assert_eq!(with.count("2 BTC"), 1);
        assert_eq!(with.count("3 BTC"), 1);
    }
    // Nothing has arrived: the plot says it waits, or cannot.
    let mut waiting = build(&compact(), &[None], SMALL, None);
    assert_eq!(overlaid(&waiting, None).0.words(), [WAITING]);
    waiting.failed = vec![true];
    assert_eq!(overlaid(&waiting, None).0.words(), [UNAVAILABLE]);
}

#[test]
fn without_a_shader_the_overlay_draws_the_bars() {
    for layout in LAYOUTS {
        let frame = four_rows(layout, true);
        let drawn = overlay::draw(&frame, None, &colors(), true);
        let record = Record::of(&drawn.painter.finish());
        // Each level's bar where the frame puts it, in its side's color at
        // the bars' opacity, before anything else is drawn.
        let faded = |color: Hsla| Hsla {
            a: color.a * shader::BAR_ALPHA,
            ..color
        };
        assert!(record.boxes.len() >= frame.levels.len());
        for (level, (rect, color)) in frame.levels.iter().zip(&record.boxes) {
            let [left, top, right, bottom] = level.rect.map(|edge| edge as f32);
            assert_eq!(*rect, [left, top, right - left, bottom - top]);
            let side = match level.side {
                Side::Bid => colors().bid,
                Side::Ask => colors().ask,
            };
            assert!(same(*color, faded(side)), "{layout:?} {level:?}");
        }
        // With one, it leaves them to it.
        let (plain, _) = overlaid(&frame, None);
        assert!(plain.boxes.len() < frame.levels.len());
    }
}

#[test]
fn the_ladder_names_the_rows_that_show_and_goes_on_past_the_book() {
    let books = [book(
        &[(100.0, 2.0)],
        &[(100.1, 4.0), (100.3, 0.0), (100.7, 5.0)],
    )];
    let config = Config {
        grouping: 0.1,
        ..compact()
    };
    let (record, _) = overlaid(&build(&config, &books, SMALL, None), None);
    let mut labels: Vec<_> = record.texts.iter().filter(|text| text.x == 6.0).collect();
    labels.sort_by(|a, b| b.y.total_cmp(&a.y));
    let named: Vec<_> = labels.iter().map(|label| label.text.as_str()).collect();
    // Packed inside the book (no row for 100.3 or 100.5), a step a row
    // past either end of it.
    assert_eq!(
        named,
        [
            "99.700", "99.800", "99.900", "100.000", "100.100", "100.700", "100.800", "100.900"
        ]
    );
    let (first, next) = (record.find("100.100"), record.find("100.700"));
    assert!(near(f64::from(first.y - next.y), ROW));
    // The row's total beside it.
    assert!(
        record
            .texts
            .iter()
            .any(|text| text.x == 74.0 && text.y == next.y)
    );

    let stepped = Config {
        all_levels: true,
        ..config.clone()
    };
    let (stepped, _) = overlaid(&build(&stepped, &books, SMALL, None), None);
    assert_eq!(stepped.count("100.200"), 1);
    let (empty, _) = overlaid(&build(&config, &[book(&[], &[])], SMALL, None), None);
    assert!(empty.texts.iter().all(|text| text.x != 6.0));
}

#[test]
fn quantities_of_two_books_sum_in_a_row_or_keep_their_lanes() {
    let books = [
        book(&[(100.0, 2.0), (99.0, 1.0)], &[(102.0, 3.0)]),
        book(&[(100.0, 4.0), (98.0, 1.0)], &[(102.0, 5.0)]),
    ];
    for layout in LAYOUTS {
        let config = Config {
            layout,
            all_levels: false,
            cumulative: true,
            ..config()
        };
        let (summed, _) = overlaid(&build(&config, &books, SMALL, None), None);
        // 100 holds 2 and 4, 102 holds 3 and 5: one text a row.
        for (text, times) in [("600", 1), ("816", 1), ("6 BTC", 1), ("8 BTC", 1)] {
            assert_eq!(summed.count(text), times, "{text} {layout:?}");
        }
        for apart in ["400", "306", "510"] {
            assert_eq!(summed.count(apart), 0, "{apart} {layout:?}");
        }
        let lanes = Config {
            separate: true,
            ..config
        };
        let (lanes, _) = overlaid(&build(&lanes, &books, SMALL, None), None);
        for text in ["200", "400", "306", "510"] {
            assert_eq!(lanes.count(text), 1, "{text} {layout:?}");
        }
        assert_eq!(lanes.count("600"), 0);
        if layout == BookLayout::Vertical {
            // The plot's 234 points halved: the second book's lane starts
            // at its middle.
            assert_eq!(lanes.find("200").x, 74.0);
            assert_eq!(lanes.find("400").x, 191.0);
        }
    }
}

#[test]
fn the_bid_ask_line_is_at_the_first_asks_edge() {
    let books = [book(&[(100.2, 2.0)], &[(101.0, 0.0), (105.1, 3.0)])];
    for layout in LAYOUTS {
        let vertical = layout == BookLayout::Vertical;
        for all_levels in [false, true] {
            let config = Config {
                layout,
                all_levels,
                grouping: 1.0,
                mid_price: false,
                divider: true,
                ..single()
            };
            let frame = build(&config, &books, SMALL, Some(103.0));
            let (record, logos) = overlaid(&frame, None);
            // Dashed, at the row edge under the asks at 106: across an
            // empty spread, past a level with no amount.
            let [line] = record.ruled(colors().accent)[..] else {
                panic!("one line: {:?}", record.lines);
            };
            assert!(line.dashed);
            let axis = if vertical { line.from.1 } else { line.from.0 };
            let edge = frame.axis_of(106.0) + if vertical { ROW / 2.0 } else { -ROW / 2.0 };
            assert!(near(f64::from(axis), edge));
            // Named after its exchange, with its logo before the name.
            assert_eq!(record.count("Bybit"), 1);
            let [logo] = &logos[..] else {
                panic!("one logo: {logos:?}");
            };
            assert_eq!((logo.exchange.as_str(), logo.size), ("bybit", 8.0));
            assert_eq!(record.find("Bybit").x, logo.x + 11.0);
        }
    }
    // No line without both sides, in order, in sight.
    let config = Config {
        mid_price: false,
        divider: true,
        ..compact()
    };
    for odd in [
        book(&[], &[]),
        book(&[(100.0, 2.0)], &[]),
        book(&[], &[(101.0, 3.0)]),
        book(&[(100.0, 2.0)], &[(100.0, 3.0)]),
        book(&[(101.0, 2.0)], &[(100.0, 3.0)]),
    ] {
        let (record, logos) = overlaid(&build(&config, &[odd], SMALL, None), None);
        assert!(record.ruled(colors().accent).is_empty());
        assert!(logos.is_empty());
    }
    let panned = build(
        &config,
        &[book(&[(100.0, 2.0)], &[(101.0, 3.0)])],
        SMALL,
        Some(200.0),
    );
    assert!(overlaid(&panned, None).0.ruled(colors().accent).is_empty());
}

#[test]
fn two_books_mid_prices_are_ruled_and_named_apart() {
    let books = [
        book(&[(100.0, 2.0)], &[(102.0, 1.0)]),
        book(&[(104.0, 3.0)], &[(106.0, 7.0)]),
    ];
    for layout in LAYOUTS {
        let config = Config {
            layout,
            grouping: 1.0,
            ..config()
        };
        let frame = build(&config, &books, SMALL, None);
        let (record, logos) = overlaid(&frame, None);
        // The primary's in the accent, the second pair's in amber, solid.
        let (first, second) = (record.ruled(colors().accent), record.ruled(colors().amber));
        assert_eq!((first.len(), second.len()), (1, 1));
        assert!(!first[0].dashed && !second[0].dashed);
        let at = |line: &Ruled| {
            f64::from(if layout == BookLayout::Vertical {
                line.from.1
            } else {
                line.from.0
            })
        };
        assert!(near(at(first[0]), frame.axis_of(101.0) + 0.5));
        assert!(near(at(second[0]), frame.axis_of(105.0) + 0.5));
        // Each price with its exchange's logo, their labels apart.
        let labelled = |price: &str, color: Hsla| {
            let found = record.texts.iter();
            found
                .filter(|text| text.text == price && same(text.color, color))
                .count()
        };
        assert_eq!(labelled("101.000", colors().accent), 1);
        assert_eq!(labelled("105.000", colors().amber), 1);
        let exchanges: Vec<_> = logos.iter().map(|logo| logo.exchange.as_str()).collect();
        assert_eq!(exchanges.len(), 2);
        assert!(exchanges.contains(&"bybit") && exchanges.contains(&"binance"));
        assert_ne!((logos[0].x, logos[0].y), (logos[1].x, logos[1].y));
    }
    // One book's price needs no logo.
    let (_, logos) = overlaid(&build(&compact(), &books[..1], SMALL, None), None);
    assert!(logos.is_empty());
}

/// The pointer on `price`'s row of `frame`.
fn pointing(frame: &Frame, price: f64) -> Option<(f64, f64)> {
    let axis = frame.axis_of(price);
    Some(if frame.config.horizontal() {
        (axis, 90.0)
    } else {
        (100.0, axis)
    })
}

#[test]
fn the_pointers_box_says_what_each_book_holds_at_its_price() {
    let usdc = MarketSelection {
        quote: "USDC".into(),
        symbol: "BTC/USDC".into(),
        ..binance()
    };
    let books = [
        book(&[(100.0, 2.0)], &[(102.0, 1.0)]),
        book(&[(98.0, 3.0)], &[(100.0, 7.0)]),
    ];
    for layout in LAYOUTS {
        let paired = Config {
            layout,
            all_levels: false,
            layers: vec![
                layer(Pair::Primary, bybit()),
                layer(Pair::Secondary, usdc.clone()),
            ],
            ..config()
        };
        let frame = build(&paired, &books, SMALL, None);
        let (record, _) = overlaid(&frame, pointing(&frame, 100.0));
        // The primary's bid and the second pair's ask at 100, side by
        // side in their books' colors, each in its own units.
        let (bid, ask) = (record.find("BID"), record.find("ASK"));
        assert!(same(bid.color, colors().bid) && same(ask.color, colors().amber));
        assert!(bid.x < ask.x);
        assert_eq!(record.find("200 USDT").x, bid.x);
        assert_eq!(record.find("700 USDC").x, ask.x);
        assert_eq!(record.find("Price level").origin, bid.origin);
        assert_eq!(record.count("100.000"), 2);
        // The pointer's line, and the box on the panel's color.
        assert_eq!(record.ruled(colors().accent).len(), 2);
        assert!(
            record
                .boxes
                .iter()
                .any(|(rect, color)| rect[0] > 0.0 && same(*color, colors().panel))
        );

        // A book with no order at that very price says so, and lends none
        // of a level near it; one that has not arrived is waited for.
        for arrived in [false, true] {
            let config = Config {
                layout,
                all_levels: false,
                ..config()
            };
            let books = [
                arrived
                    .then(|| book(&[(99.0, 2.0)], &[(103.0, 1.0)]))
                    .flatten(),
                book(&[(100.0, 7.0)], &[(102.0, 1.0)]),
            ];
            let frame = build(&config, &books, SMALL, None);
            let (record, _) = overlaid(&frame, pointing(&frame, 100.0));
            let name = record.find("Bybit");
            let column: Vec<_> = record
                .texts
                .iter()
                .filter(|text| text.x == name.x && text.origin == name.origin)
                .map(|text| text.text.as_str())
                .collect();
            let said = if arrived { "No orders" } else { "Waiting" };
            assert_eq!(column, ["Bybit", "BTC/USDT", said, "—", "—", "—"]);
            assert_eq!((record.count("7 BTC"), record.count("2 BTC")), (2, 0));
        }
    }
}

#[test]
fn the_pointers_box_shows_only_on_a_level() {
    for layout in LAYOUTS {
        // Both sides of one book at one price keep a column each.
        let config = Config {
            layout,
            ..compact()
        };
        let frame = build(
            &config,
            &[book(&[(100.0, 2.0)], &[(100.0, 3.0)])],
            SMALL,
            None,
        );
        let (record, _) = overlaid(&frame, pointing(&frame, 100.0));
        let (bid, ask) = (record.find("BID"), record.find("ASK"));
        assert!(bid.x < ask.x);
        assert!(same(bid.color, colors().bid) && same(ask.color, colors().ask));
        assert_eq!(record.find("2 BTC").x, bid.x);
        assert_eq!(record.find("3 BTC").x, ask.x);

        // An empty row of the ladder, and a pointer that left, show none.
        let config = Config {
            layout,
            grouping: 1.0,
            ..single()
        };
        let frame = build(
            &config,
            &[book(&[(100.0, 2.0)], &[(102.0, 3.0)])],
            SMALL,
            None,
        );
        for pointer in [pointing(&frame, 101.0), None] {
            let (record, _) = overlaid(&frame, pointer);
            assert_eq!(record.count("Price level"), 0);
            assert!(record.boxes.iter().all(|(rect, _)| rect[0] <= 0.0));
        }
    }
}

/// A workspace of one order book, with its data services as the shell
/// provides them.
#[component]
fn Harness() -> impl IntoElement {
    let services = DataServices::provide_with(FerrisConfig {
        http_base: FERRIS.to_owned(),
        ws_url: "ws://127.0.0.1:9/v1/ws".to_owned(),
        ..FerrisConfig::production()
    });
    provide_context(Clock::restore());
    let mut saved = StoredWorkspace::default();
    layout::normalize(&mut saved);
    layout::spawn(&mut saved, WidgetKind::OrderBook);
    let workspace = signal(saved);
    let picker = Picker::new();
    services
        .sessions
        .follow(move || layout::widget_ids(&workspace.get()));
    view()
        .size_full()
        .flex_col()
        .child(layout::workspace(workspace, picker))
        .child(picker.view(workspace))
}

#[test]
fn an_order_book_shows_its_book_and_follows_its_settings() {
    let network = Arc::new(StubNetwork::new([(
        format!("POST {FERRIS}/v1/fetchOrderBook"),
        StubRoute::Handler(Arc::new(stub_book)),
    )]));
    // The books asked for, as their request bodies.
    let asked = {
        let network = network.clone();
        move || -> Vec<Value> {
            let requests = network.requests().into_iter();
            requests
                .filter(|request| request.url.ends_with("/v1/fetchOrderBook"))
                .map(|request| match &request.body {
                    HttpBody::Bytes(bytes) => serde_json::from_slice(bytes).unwrap_or_default(),
                    _ => Value::Null,
                })
                .collect()
        }
    };
    let mut config = EngineConfig::new(nsh_testing::windowed_root::<Harness>());
    config.viewport = Size::new(px(1280.0), px(800.0));
    config.storage = Some(MemoryStorage::new([]));
    config.network = Some(network.clone());
    config.environment.os = Os::Windows;
    config.environment.idiom = Idiom::Desktop;
    let mut app = Headless::start(config).unwrap();
    // What the overlay wrote last, and the uniforms of every tile drawn.
    let written = Arc::new(Mutex::new(Vec::<String>::new()));
    let layers = Arc::new(Mutex::new(Vec::<Vec<f32>>::new()));
    {
        let (written, layers) = (written.clone(), layers.clone());
        app.observe(move |_, batch| {
            for item in &batch.items {
                match item {
                    MountItem::Canvas { drawing, .. } => {
                        let record = Record::of(drawing);
                        if !record.texts.is_empty() {
                            *written.lock().unwrap() =
                                record.words().into_iter().map(str::to_owned).collect();
                        }
                    }
                    MountItem::Shader {
                        layer: Some(layer), ..
                    } => {
                        let words = layer.uniforms.iter().map(|word| f32::from_bits(*word));
                        layers.lock().unwrap().push(words.collect());
                    }
                    _ => {}
                }
            }
        });
    }
    let writes = {
        let written = written.clone();
        move |text: &str| written.lock().unwrap().iter().any(|word| word == text)
    };
    let newest = |word: usize| {
        layers
            .lock()
            .unwrap()
            .last()
            .map(|words: &Vec<f32>| words[word])
    };

    // Bybit's BTC/USDT by default, 200 levels a side.
    app.wait_for("BTC/USDT").unwrap();
    app.wait_until("the book", WAIT, |_| writes("100.250"))
        .unwrap();
    let first = asked();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0]["limit"], 200);
    assert_eq!(first[0]["symbol"], "BTC/USDT:USDT");
    assert_eq!(
        first[0]["params"],
        json!({ "category": "linear", "levels": 200, "amountUnit": "base" })
    );
    // Its prices in the column, each row's total beside it, the mid price
    // on its line; no running amounts yet.
    for text in [
        "101.000", "100.500", "100.000", "99.500", "200", "398", "101", "303",
    ] {
        assert!(writes(text), "{text}");
    }
    assert!(!writes("2 BTC") && !writes(WAITING));
    // One tile of bars, prices running down it.
    assert_eq!(newest(2), Some(0.0));

    // The top bar's selects: a grouping merges the asks into one row.
    app.press("Group").unwrap();
    app.press("10").unwrap();
    app.wait_until("grouped rows", WAIT, |_| writes("110.000"))
        .unwrap();
    assert!(writes("90.000") && writes("404") && !writes("303"));
    // Another depth is another book.
    app.press("Depth").unwrap();
    app.press("50").unwrap();
    app.wait_until("the shallower book", WAIT, |_| asked().len() == 2)
        .unwrap();
    assert_eq!(asked()[1]["limit"], 50);

    // The pointer's box names the level under it.
    assert!(!writes("Price level"));
    app.hover("id:order-book-plot").unwrap();
    app.wait_until("the pointer's box", WAIT, |_| writes("Price level"))
        .unwrap();
    assert!(writes("BID") && writes("Amount") && writes("Bybit"));

    // The settings panel: running amounts, then the ladder across.
    app.press("Widget settings").unwrap();
    app.wait_for("id:order-book-settings").unwrap();
    app.press("Show Cumulative Amount").unwrap();
    app.wait_until("running amounts", WAIT, |_| writes("2 BTC"))
        .unwrap();
    app.press("Layout").unwrap();
    app.press("Horizontal").unwrap();
    app.wait_until("the ladder across", WAIT, |_| newest(2) == Some(1.0))
        .unwrap();
    assert!(writes("110.000"));

    // The top bar's + opens the market picker for a second pair.
    app.press("Track another pair").unwrap();
    app.wait_for("Search for symbol").unwrap();
    app.press_key("Escape").unwrap();
    app.wait_for_gone("Search for symbol", WAIT).unwrap();
    assert!(app.shows("BTC/USDT"));
}

/// Answers `fetchOrderBook` as Ferris does for a market whose contracts it
/// cannot convert: base amounts are refused, native ones are contracts,
/// and the book is an RFQ market's.
fn stub_unconverted(request: nsh_engine::HttpRequest, reply: nsh_testing::StubReply) {
    let body: Value = match &request.body {
        HttpBody::Bytes(bytes) => serde_json::from_slice(bytes).unwrap_or_default(),
        _ => Value::Null,
    };
    let mut response = nsh_testing::StubResponse::default();
    let answer = if body["params"]["amountUnit"] == "base" {
        response.status = 400;
        json!({
            "code": "UNSUPPORTED_FEATURE",
            "message": "AMOUNT_NORMALIZATION_UNAVAILABLE: unverified_inverse_contract_face_size \
                        (bybit:inverse_book)",
        })
    } else {
        response.status = 200;
        json!({
            "bids": [[100.0, 2.0]],
            "asks": [[100.5, 1.0]],
            "timestamp": 1_700_000_000_000_i64,
            "amountUnit": "contracts",
            "amountCurrency": null,
            "liquidityKind": "indicative",
        })
    };
    response.body = serde_json::to_vec(&answer).expect("an answer is JSON");
    reply.send(response);
}

#[test]
fn a_book_ferris_cannot_convert_is_asked_for_natively_and_says_so() {
    let network = Arc::new(StubNetwork::new([(
        format!("POST {FERRIS}/v1/fetchOrderBook"),
        StubRoute::Handler(Arc::new(stub_unconverted)),
    )]));
    let mut config = EngineConfig::new(nsh_testing::windowed_root::<Harness>());
    config.viewport = Size::new(px(1280.0), px(800.0));
    config.storage = Some(MemoryStorage::new([]));
    config.network = Some(network.clone());
    config.environment.os = Os::Windows;
    config.environment.idiom = Idiom::Desktop;
    let mut app = Headless::start(config).unwrap();

    // The notices name the book, and each tag (written in capitals) what
    // it cannot be read as.
    app.wait_for("CONTRACT QUANTITIES").unwrap();
    assert!(app.shows("INDICATIVE BOOK"));
    assert!(app.shows("Primary: Bybit BTC/USDT"));
    // Base amounts first, then the venue's own once Ferris refused them.
    let asked: Vec<Value> = network
        .requests()
        .into_iter()
        .filter(|request| request.url.ends_with("/v1/fetchOrderBook"))
        .map(|request| match &request.body {
            HttpBody::Bytes(bytes) => serde_json::from_slice(bytes).unwrap_or_default(),
            _ => Value::Null,
        })
        .collect();
    assert_eq!(asked.len(), 2);
    assert_eq!(asked[0]["params"]["amountUnit"], "base");
    assert_eq!(asked[1]["params"].get("amountUnit"), None);
}
