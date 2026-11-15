use std::{
    rc::Rc,
    sync::{Arc, Mutex},
};

use nsh_core::{
    MountItem,
    element::Root,
    platform::{Idiom, Os},
    prelude::*,
    shader::ShaderLayer,
};
use nsh_engine::{EngineConfig, HttpBody};
use nsh_testing::{Headless, MemoryStorage, StubNetwork, StubRoute, WAIT};
use serde_json::{Value, json};

use super::{
    frame::{Config, Frame, MAX_ZOOM, MIN_ZOOM, places},
    overlay::{self, depth_text},
    session::Window,
    settings::{Blend, DepthChartSettings, Unit},
    shader::{self, COLUMN_CAPACITY, FILL},
};
use crate::{
    data::{
        DataServices,
        ferris::{FerrisConfig, MarketSelection, OrderBook},
    },
    layout,
    shell::clock::Clock,
    storage::StoredWorkspace,
    widgets::{
        WidgetKind,
        common::books::{
            BookLayout, BookSettings, Grouping, Layer, Logo, Pair, Side, UNAVAILABLE, WAITING,
            testing::{
                CLEAR, FERRIS, GREEN, RED, Record, book, colors, drawn, palette, pixel, same,
                stub_book,
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

fn layer(pair: Pair, market: MarketSelection) -> Layer {
    Layer {
        pair,
        market: Rc::new(market),
        key: pair.id().to_owned(),
    }
}

/// Two books at raw prices, drawn over one another, prices along the
/// bottom.
fn config() -> Config {
    Config {
        layout: BookLayout::Horizontal,
        depth: 200,
        grouping: 0.0,
        blend: Blend::Overlaid,
        unit: Unit::Amount,
        mid_price: true,
        layers: vec![
            layer(Pair::Primary, bybit()),
            layer(Pair::Secondary, binance()),
        ],
        warning: None,
    }
}

/// The primary book alone.
fn single() -> Config {
    Config {
        layers: vec![layer(Pair::Primary, bybit())],
        ..config()
    }
}

/// A book about 100.5: 1, 2 and 4 deep through its bids at 100, 99 and
/// 98; 2, 4 and 8 deep through its asks at 101, 102 and 104.
fn steps() -> Option<Rc<OrderBook>> {
    book(
        &[(100.0, 1.0), (99.0, 1.0), (98.0, 2.0)],
        &[(101.0, 2.0), (102.0, 2.0), (104.0, 4.0)],
    )
}

fn build(
    config: &Config,
    books: &[Option<Rc<OrderBook>>],
    (width, height): (f64, f64),
    zoom: Option<f64>,
) -> Frame {
    Frame::build(Rc::new(config.clone()), books, width, height, zoom, None)
}

fn near(left: f64, right: f64) -> bool {
    (left - right).abs() < 1e-6
}

/// Whether a column reaches as far as `reaches` says, series by series.
fn reaches(column: &[f32; 4], reaches: [f64; 4]) -> bool {
    column
        .iter()
        .zip(reaches)
        .all(|(reach, wanted)| near(f64::from(*reach), wanted))
}

/// `steps()` on a plot 200 points along the prices and 100 across: with
/// prices along the bottom, above their 22 point strip; with prices down
/// the pane, beside their 66 point column.
fn stepped(layout: BookLayout) -> Frame {
    let config = Config { layout, ..single() };
    let size = match layout {
        BookLayout::Horizontal => (200.0, 122.0),
        BookLayout::Vertical => (166.0, 200.0),
    };
    build(&config, &[steps()], size, None)
}

const LAYOUTS: [BookLayout; 2] = [BookLayout::Horizontal, BookLayout::Vertical];

#[test]
fn settings_save_only_what_differs_and_read_leniently() {
    let saved = |settings: &DepthChartSettings| serde_json::to_value(settings).unwrap();
    let defaults = DepthChartSettings::default();
    assert_eq!(saved(&defaults), json!({}));
    assert_eq!(defaults.books().market(Pair::Primary), Some(bybit()));
    assert_eq!(defaults.books().market(Pair::Secondary), None);
    assert_eq!(
        (defaults.books().depth(), defaults.books().grouping()),
        (200, Grouping::Raw)
    );
    assert_eq!(
        (defaults.blend(), defaults.unit(), defaults.layout()),
        (Blend::Overlaid, Unit::Amount, BookLayout::Horizontal)
    );
    assert!(defaults.mid_price());

    let settings = defaults
        .clone()
        .with_market(Pair::Secondary, Some(binance()))
        .with_depth(50)
        .with_grouping(Grouping::Ten)
        .with_blend(Blend::Stacked)
        .with_unit(Unit::Value)
        .with_mid_price(false)
        .with_layout(BookLayout::Vertical);
    let value = saved(&settings);
    assert_eq!(value["depth"], 50);
    assert_eq!(value["priceGrouping"], 10);
    assert_eq!(value["pairMode"], "stacked");
    assert_eq!(value["depthUnit"], "value");
    assert_eq!(value["showMidPriceLine"], false);
    assert_eq!(value["layout"], "vertical");
    assert_eq!(value["secondaryMarket"]["exchange"], "binance");
    assert_eq!(value.get("primaryMarket"), None);
    assert_eq!(DepthChartSettings::from(value), settings);
    // Back at their defaults, they are left out again.
    let back = settings
        .with_blend(Blend::Overlaid)
        .with_unit(Unit::Amount)
        .with_mid_price(true)
        .with_layout(BookLayout::Horizontal);
    for name in ["pairMode", "depthUnit", "showMidPriceLine", "layout"] {
        assert_eq!(saved(&back).get(name), None, "{name}");
    }

    // The old depth chart's one market is the primary, unless a primary
    // was saved since (cleared, here).
    let read = DepthChartSettings::from;
    let market = serde_json::to_value(binance()).unwrap();
    let old = read(json!({ "market": market, "depth": 50, "showMidPriceLine": false }));
    assert_eq!(old.books().market(Pair::Primary), Some(binance()));
    assert_eq!((old.books().depth(), old.mid_price()), (50, false));
    assert_eq!(saved(&old)["primaryMarket"]["exchange"], "binance");
    let cleared = read(json!({ "market": market, "primaryMarket": null }));
    assert_eq!(cleared.books().market(Pair::Primary), None);

    // What it cannot read is its default.
    let damaged = read(json!({
        "pairMode": "sideways",
        "depthUnit": 3,
        "showMidPriceLine": "no",
        "layout": "diagonal",
        "market": "BTC",
    }));
    assert_eq!(damaged, DepthChartSettings::default());
    assert_eq!(read(json!([1, 2])), DepthChartSettings::default());
}

#[test]
fn two_books_are_warned_about_when_they_cannot_be_read_together() {
    let pairs = |settings: &DepthChartSettings| -> Vec<Pair> {
        settings.layers().iter().map(|layer| layer.pair).collect()
    };
    let alone = DepthChartSettings::default();
    assert_eq!(
        (pairs(&alone), alone.warning()),
        (vec![Pair::Primary], None)
    );
    let two = alone.clone().with_market(Pair::Secondary, Some(binance()));
    assert_eq!(pairs(&two), [Pair::Primary, Pair::Secondary]);
    assert_eq!(two.warning(), None);

    // The primary's book again is not drawn twice.
    let again = alone.clone().with_market(Pair::Secondary, Some(bybit()));
    assert_eq!(pairs(&again), [Pair::Primary]);
    assert_eq!(
        again.warning(),
        Some("Pair 2 duplicates Primary and is not rendered.")
    );

    // Amounts of two base assets add up to nothing; their values in one
    // quote asset do.
    let ether = MarketSelection {
        symbol: "ETH/USDT".into(),
        base: "ETH".into(),
        request_symbol: "ETH/USDT:USDT".into(),
        exchange_symbol: "ETHUSDT".into(),
        ..bybit()
    };
    let mixed = alone.with_market(Pair::Secondary, Some(ether));
    assert_eq!(
        mixed.warning(),
        Some("Pairs have different base assets; amounts are not normalized.")
    );
    assert_eq!(mixed.with_unit(Unit::Value).warning(), None);
}

#[test]
fn a_side_is_a_curve_of_what_rests_through_each_price() {
    let frame = stepped(BookLayout::Horizontal);
    let through = |side: Side, price: f64| {
        frame
            .through(0, side, price)
            .map(|step| (step.price, step.amount, step.value))
    };
    // Nothing nearer the asks than the best bid; then each level's
    // running amount and value, the last one's past it.
    assert_eq!(through(Side::Bid, 100.1), None);
    assert_eq!(through(Side::Bid, 100.0), Some((100.0, 1.0, 100.0)));
    assert_eq!(through(Side::Bid, 99.5), Some((100.0, 1.0, 100.0)));
    assert_eq!(through(Side::Bid, 99.0), Some((99.0, 2.0, 199.0)));
    assert_eq!(through(Side::Bid, 98.0), Some((98.0, 4.0, 395.0)));
    assert_eq!(through(Side::Bid, 50.0), Some((98.0, 4.0, 395.0)));
    assert_eq!(through(Side::Ask, 100.9), None);
    assert_eq!(through(Side::Ask, 101.0), Some((101.0, 2.0, 202.0)));
    assert_eq!(through(Side::Ask, 103.9), Some((102.0, 4.0, 406.0)));
    assert_eq!(through(Side::Ask, 500.0), Some((104.0, 8.0, 822.0)));
    // A book that has not arrived has no curve to be on.
    assert_eq!(through(Side::Bid, 1.0).is_some(), frame.loaded[0]);
    let waiting = build(&single(), &[None], (200.0, 122.0), None);
    assert_eq!(waiting.through(0, Side::Bid, 1.0), None);
    assert_eq!(
        (waiting.loaded.as_slice(), waiting.has_prices),
        (&[false][..], false)
    );

    // Groups take prices outward and hold what their levels do: the bids
    // at 100 and 99 in 90's, the asks at 101 and 102 in 110's.
    let grouped = Config {
        grouping: 10.0,
        ..single()
    };
    let grouped = build(&grouped, &[steps()], (200.0, 122.0), None);
    let curve = |side: Side| -> Vec<(f64, f64, f64)> {
        let curve = grouped.curves.iter().find(|curve| curve.side == side);
        let steps = curve
            .map(|curve| curve.steps.as_slice())
            .unwrap_or_default();
        steps
            .iter()
            .map(|step| (step.price, step.amount, step.value))
            .collect()
    };
    assert_eq!(curve(Side::Bid), [(100.0, 1.0, 100.0), (90.0, 4.0, 395.0)]);
    assert_eq!(curve(Side::Ask), [(110.0, 8.0, 822.0)]);
    // The mid price is the book's, whatever its groups.
    assert_eq!((grouped.mid, grouped.center), (Some(100.5), 100.5));

    // Only the best `depth` levels a side count.
    let shallow = Config {
        depth: 2,
        ..single()
    };
    let shallow = build(&shallow, &[steps()], (200.0, 122.0), None);
    assert_eq!(
        shallow.through(0, Side::Bid, 1.0).map(|step| step.amount),
        Some(2.0)
    );
}

#[test]
fn the_window_is_about_the_mid_price_as_far_as_every_side_is_loaded() {
    for layout in LAYOUTS {
        let frame = stepped(layout);
        assert!(frame.has_prices);
        assert_eq!((frame.mid, frame.center), (Some(100.5), 100.5));
        // The bids end 2.5 from the center, the asks 3.5: every side is
        // loaded through 2.5.
        assert!(near(frame.span, 2.5), "{}", frame.span);
        assert_eq!((frame.length(), frame.breadth()), (200.0, 100.0));

        assert!(near(frame.axis_of(100.5), 100.0));
        for price in [98.0, 99.25, 100.5, 103.0] {
            assert!(near(frame.price_at(frame.axis_of(price)), price));
            assert!(frame.shows(frame.axis_of(price)));
        }
        assert!(!frame.shows(frame.axis_of(97.9)) && !frame.shows(frame.axis_of(103.1)));
        match layout {
            // Prices rise to the right, depth up from the strip's edge.
            BookLayout::Horizontal => {
                assert_eq!(
                    (
                        frame.plot_left,
                        frame.plot_top,
                        frame.plot_right,
                        frame.plot_bottom
                    ),
                    (0.0, 0.0, 200.0, 100.0)
                );
                assert!(near(frame.axis_of(98.0), 0.0) && near(frame.axis_of(103.0), 200.0));
                assert!(near(frame.reach_of(0.0), 100.0) && near(frame.reach_of(frame.top), 0.0));
                assert_eq!(
                    frame.pointed(40.0, 50.0).map(|price| near(price, 99.0)),
                    Some(true)
                );
                assert_eq!(frame.pointed(40.0, 110.0), None);
            }
            // Prices rise up the pane, depth right from the column's edge.
            BookLayout::Vertical => {
                assert_eq!(
                    (
                        frame.plot_left,
                        frame.plot_top,
                        frame.plot_right,
                        frame.plot_bottom
                    ),
                    (66.0, 0.0, 166.0, 200.0)
                );
                assert_eq!(frame.price_column, 66.0);
                assert!(near(frame.axis_of(103.0), 0.0) && near(frame.axis_of(98.0), 200.0));
                assert!(near(frame.reach_of(0.0), 66.0) && near(frame.reach_of(frame.top), 166.0));
                assert_eq!(
                    frame.pointed(100.0, 160.0).map(|price| near(price, 99.0)),
                    Some(true)
                );
                assert_eq!(frame.pointed(30.0, 160.0), None);
            }
        }
    }

    // One side alone is centered on its best price; no price at all stays
    // where the last frame was.
    let bids = [book(&[(100.0, 1.0), (99.0, 1.0)], &[])];
    let one_sided = build(&single(), &bids, (200.0, 122.0), None);
    assert_eq!((one_sided.mid, one_sided.center), (None, 100.0));
    assert!(one_sided.has_prices && near(one_sided.span, 1.0));
    let held = Frame::build(Rc::new(single()), &[None], 200.0, 122.0, None, Some(64.0));
    assert_eq!((held.has_prices, held.center), (false, 64.0));
    assert!(held.columns.iter().all(|column| *column == [0.0; 4]));

    // Two books are centered between the highest bid and the lowest ask
    // of both, and looked at as far as the nearest side goes.
    let books = [
        steps(),
        book(
            &[(100.25, 1.0), (100.0, 1.0)],
            &[(100.75, 1.0), (101.5, 1.0)],
        ),
    ];
    let both = build(&config(), &books, (200.0, 122.0), None);
    assert_eq!((both.mid, both.center), (Some(100.5), 100.5));
    assert_eq!(both.mids, [Some(100.5), Some(100.5)]);
    assert!(near(both.span, 0.5), "{}", both.span);
    // A window is never narrower or wider than a zoom may be.
    let tight = [book(&[(100.0, 1.0)], &[(100.0001, 1.0)])];
    let tight = build(&single(), &tight, (200.0, 122.0), None);
    assert!(near(tight.span, tight.center * MIN_ZOOM));
    let wide = [book(
        &[(100.0, 1.0), (1.0, 1.0)],
        &[(101.0, 1.0), (900.0, 1.0)],
    )];
    let wide = build(&single(), &wide, (200.0, 122.0), None);
    assert!(near(wide.span, wide.center * MAX_ZOOM));
}

/// The next frame of `window` on the tests' plot, no topic refused.
fn next(window: &mut Window, config: &Rc<Config>, books: &[Option<Rc<OrderBook>>]) -> Frame {
    window.frame(config, books, Vec::new(), 200.0, 122.0)
}

#[test]
fn a_zoom_keeps_its_share_of_the_price_until_the_window_is_fitted_again() {
    let alone = Rc::new(single());
    let mut window = Window::default();
    let frame = next(&mut window, &alone, &[steps()]);
    assert!(near(frame.span, 2.5));

    // Twice as wide, about the same center; the book is loaded as far.
    assert!(window.zoom(&frame, 2.0));
    let zoomed = next(&mut window, &alone, &[steps()]);
    assert!(near(zoomed.span, 5.0) && near(zoomed.center, 100.5));
    assert!(near(zoomed.fit, 2.5));
    // The price doubles: the window is as wide a share of it.
    let doubled = [book(&[(200.0, 1.0), (199.0, 1.0)], &[(202.0, 1.0)])];
    let moved = next(&mut window, &alone, &doubled);
    assert!(near(moved.center, 201.0) && near(moved.span, 10.0));
    // Never past what a zoom may be, either way.
    assert!(window.zoom(&moved, 1e9));
    let widest = next(&mut window, &alone, &doubled);
    assert!(near(widest.span, 201.0 * MAX_ZOOM));
    assert!(window.zoom(&widest, 1e-12));
    let narrowest = next(&mut window, &alone, &doubled);
    assert!(near(narrowest.span, 201.0 * MIN_ZOOM));

    // A frame without prices stays where the last one was, and has no
    // window to zoom.
    let waiting = next(&mut window, &alone, &[None]);
    assert_eq!((waiting.has_prices, waiting.center), (false, 201.0));
    assert!(!window.zoom(&waiting, 2.0));
    assert!(!window.zoom(&moved, 0.0) && !window.zoom(&moved, f64::NAN));

    // Fitted again, it goes as far as the book is loaded.
    window.fit();
    let fitted = next(&mut window, &alone, &[steps()]);
    assert!(near(fitted.span, 2.5));

    // Other books, or other levels of them, are another window; what is
    // drawn of the same levels is not.
    let other = Config {
        layers: vec![layer(Pair::Secondary, binance())],
        ..single()
    };
    assert!(single().refitted_by(&other) && single().refitted_by(&config()));
    let regrouped = Config {
        grouping: 10.0,
        ..single()
    };
    let shallower = Config {
        depth: 50,
        ..single()
    };
    assert!(single().refitted_by(&regrouped) && single().refitted_by(&shallower));
    let turned = Config {
        layout: BookLayout::Vertical,
        blend: Blend::Stacked,
        unit: Unit::Value,
        mid_price: false,
        ..single()
    };
    assert!(!single().refitted_by(&turned));
}

#[test]
fn a_fitted_window_is_kept_while_the_books_stay_about_as_deep() {
    let alone = Rc::new(single());
    let mut window = Window::default();
    let share = 2.5 / 100.5;
    assert!(near(next(&mut window, &alone, &[steps()]).span, 2.5));

    // The book loads further, then less far: the window is as wide, so
    // the curves are where they were.
    let further = [book(
        &[(100.0, 1.0), (97.0, 1.0)],
        &[(101.0, 2.0), (104.0, 4.0)],
    )];
    let frame = next(&mut window, &alone, &further);
    assert!(near(frame.fit, 3.5) && near(frame.span, 2.5));
    let nearer = [book(
        &[(100.0, 1.0), (99.0, 1.0)],
        &[(101.0, 2.0), (104.0, 4.0)],
    )];
    let frame = next(&mut window, &alone, &nearer);
    assert!(near(frame.fit, 1.5) && near(frame.span, 2.5));
    // The price moves: the window is as wide a share of it.
    let risen = [book(
        &[(102.0, 1.0), (100.0, 1.0)],
        &[(103.0, 2.0), (106.0, 4.0)],
    )];
    let frame = next(&mut window, &alone, &risen);
    assert!(near(frame.center, 102.5) && near(frame.span / frame.center, share));

    // A book loaded through less than half of it no longer fills it: it
    // is fitted again, at once.
    let shallow = [book(&[(100.0, 1.0)], &[(101.0, 2.0)])];
    assert!(near(next(&mut window, &alone, &shallow).span, 0.5));
    // That fit is kept in turn, until the book is loaded through more
    // than twice it.
    let deeper = [book(
        &[(100.0, 1.0), (99.6, 1.0)],
        &[(101.0, 2.0), (101.4, 2.0)],
    )];
    let frame = next(&mut window, &alone, &deeper);
    assert!(near(frame.fit, 0.9) && near(frame.span, 0.5));
    assert!(near(next(&mut window, &alone, &[steps()]).span, 2.5));

    // What the user zoomed to is theirs: no book fits it again.
    let frame = next(&mut window, &alone, &[steps()]);
    assert!(window.zoom(&frame, 0.5));
    assert!(near(next(&mut window, &alone, &shallow).span, 1.25));

    // Nothing is kept until every book has arrived: the first one's fit
    // would hold the second's (loaded through 2 where the first is
    // through 2.5), and what both fit is kept from then on.
    let two = Rc::new(config());
    let mut window = Window::default();
    assert!(near(next(&mut window, &two, &[steps(), None]).span, 2.5));
    let second = book(&[(100.0, 1.0), (98.5, 1.0)], &[(101.0, 1.0), (102.5, 1.0)]);
    assert!(near(next(&mut window, &two, &[steps(), second]).span, 2.0));
    let second = book(&[(100.0, 1.0), (97.5, 1.0)], &[(101.0, 1.0), (103.5, 1.0)]);
    let frame = next(&mut window, &two, &[steps(), second]);
    assert!(near(frame.fit, 2.5) && near(frame.span, 2.0));
    // A book whose topic was refused never will: the other's fit is kept.
    let mut window = Window::default();
    let refused = || vec![false, true];
    let frame = window.frame(&two, &[steps(), None], refused(), 200.0, 122.0);
    assert!(near(frame.span, 2.5));
    assert_eq!(frame.failed, [false, true]);
    let books = [further[0].clone(), None];
    let frame = window.frame(&two, &books, refused(), 200.0, 122.0);
    assert!(near(frame.fit, 3.5) && near(frame.span, 2.5));
}

#[test]
fn the_scale_is_a_round_depth_just_above_the_deepest_shown() {
    // The window shows the bids through 98 (4 deep) and the asks through
    // 102 (4 deep): rules a whole unit apart, the first one clear of 4.
    let frame = stepped(BookLayout::Horizontal);
    assert_eq!((frame.rule, frame.top), (1.0, 5.0));
    assert_eq!(frame.rules().collect::<Vec<_>>(), [1.0, 2.0, 3.0, 4.0, 5.0]);
    // In the quote asset the same book is 406 deep at most.
    let valued = Config {
        unit: Unit::Value,
        ..single()
    };
    let valued = build(&valued, &[steps()], (200.0, 122.0), None);
    assert_eq!((valued.rule, valued.top), (100.0, 500.0));
    // Zoomed out, the asks' last level counts too.
    let wide = build(&single(), &[steps()], (200.0, 122.0), Some(0.05));
    assert_eq!((wide.rule, wide.top), (2.5, 10.0));

    for (deepest, rule, top) in [
        (0.9, 0.25, 1.0),
        (7.0, 2.0, 8.0),
        (1_234.0, 250.0, 1_500.0),
        (0.004, 0.001, 0.005),
    ] {
        let books = [book(&[(100.0, deepest)], &[(101.0, deepest)])];
        let frame = build(&single(), &books, (200.0, 122.0), None);
        assert!(
            near(frame.rule, rule) && near(frame.top, top),
            "{deepest}: {} {}",
            frame.rule,
            frame.top
        );
        let rules = frame.rules().count();
        assert!((3..=7).contains(&rules), "{deepest}: {rules} rules");
    }
    // Nothing to scale by is still a scale.
    let empty = build(&single(), &[book(&[], &[])], (200.0, 122.0), None);
    assert_eq!((empty.rule, empty.top), (0.25, 1.0));
}

/// `steps()` with every price `by` higher.
fn risen(by: f64) -> Option<Rc<OrderBook>> {
    book(
        &[(100.0 + by, 1.0), (99.0 + by, 1.0), (98.0 + by, 2.0)],
        &[(101.0 + by, 2.0), (102.0 + by, 2.0), (104.0 + by, 4.0)],
    )
}

#[test]
fn prices_are_marked_at_places_that_never_move() {
    // A mark at the plot's middle and every 88 points from it, short of
    // its edges; 1.1 of price apart, they are named to a decimal.
    let frame = build(&single(), &[steps()], (400.0, 122.0), None);
    let marks = frame.marks();
    assert_eq!(marks.at, [24.0, 112.0, 200.0, 288.0, 376.0]);
    assert_eq!(marks.places, 1);
    // The price moves: the marks are where they were, and each names the
    // price that is there now.
    let moved = build(&single(), &[risen(0.37)], (400.0, 122.0), None);
    assert_eq!(moved.marks(), marks);
    let named = |frame: &Frame| -> Vec<(f32, String)> {
        let (record, _) = overlaid(frame, None);
        let strip = record.texts.iter().filter(|text| text.y == 111.5);
        strip.map(|text| (text.x, text.text.clone())).collect()
    };
    let names = |names: [&str; 5]| -> Vec<(f32, String)> {
        let at = [24.0, 112.0, 200.0, 288.0, 376.0];
        at.into_iter().zip(names.map(str::to_owned)).collect()
    };
    assert_eq!(
        named(&frame),
        names(["98.3", "99.4", "100.5", "101.6", "102.7"])
    );
    assert_eq!(
        named(&moved),
        names(["98.7", "99.8", "100.9", "102.0", "103.1"])
    );
    // So do the rules along the plot: where they were.
    let rules = |frame: &Frame| -> Vec<(f32, f32)> {
        let (record, _) = overlaid(frame, None);
        let ruled = record.ruled(faded(colors().grid, 0.5));
        let along = ruled.iter().filter(|line| line.from.1 == 0.0);
        along.map(|line| line.from).collect()
    };
    assert_eq!(rules(&frame).len(), 5);
    assert_eq!(rules(&frame), rules(&moved));

    // A wider plot has the same marks about its middle, and more of them
    // where there is room; none on its edges.
    let about = |width: f64| -> Vec<f64> {
        let frame = build(&single(), &[steps()], (width, 122.0), None);
        let marks = frame.marks().at;
        marks.iter().map(|at| at - width / 2.0).collect()
    };
    assert_eq!(about(488.0), [-176.0, -88.0, 0.0, 88.0, 176.0]);
    assert_eq!(about(352.0), [-88.0, 0.0, 88.0]);
    assert_eq!(about(540.0).len(), 7);
    assert_eq!(about(60.0), [0.0]);

    // Down the pane a name needs less room: a mark every 44 points.
    let down = stepped(BookLayout::Vertical).marks();
    assert_eq!(down.at, [12.0, 56.0, 100.0, 144.0, 188.0]);
    assert_eq!(down.places, 1);

    // Names tell a fifth of what is between two marks, no more.
    for (apart, decimals) in [
        (250.0, 0),
        (12.5, 0),
        (6.875, 0),
        (3.0, 1),
        (1.1, 1),
        (0.4, 2),
        (0.000_1, 5),
        (0.0, 0),
        (f64::NAN, 0),
    ] {
        assert_eq!(places(apart), decimals, "{apart}");
    }
}

#[test]
fn columns_carry_how_far_each_series_reaches_and_tiles_cut_them() {
    let frame = stepped(BookLayout::Horizontal);
    // A column a point, and one before the plot and one after it.
    assert_eq!(frame.columns.len(), 202);
    let [tile] = frame.tiles.as_slice() else {
        panic!("200 points are one tile: {}", frame.tiles.len());
    };
    assert_eq!(
        (tile.offset, tile.length, tile.first, tile.count),
        (0.0, 200.0, 0, 202)
    );
    // Prices rise along the columns, 40 a unit of price from 98: the bids
    // through 98, 99 and 100 (4, 2 and 1 of the scale's 5), nothing across
    // the spread, then the asks through 101 and 102.
    let bids = |reach: f64| [reach, 0.0, 0.0, 0.0];
    let asks = |reach: f64| [0.0, reach, 0.0, 0.0];
    for (columns, wanted) in [
        (0..=0, bids(0.8)),
        (1..=40, bids(0.4)),
        (41..=80, bids(0.2)),
        (81..=120, [0.0; 4]),
        (121..=160, asks(0.4)),
        (161..=201, asks(0.8)),
    ] {
        for column in columns {
            assert!(reaches(&frame.columns[column], wanted), "column {column}");
        }
    }
    // Down the pane they run from the highest price.
    let down = stepped(BookLayout::Vertical);
    assert_eq!(down.columns.len(), 202);
    for (columns, wanted) in [
        (0..=40, asks(0.8)),
        (41..=80, asks(0.4)),
        (81..=120, [0.0; 4]),
        (121..=160, bids(0.2)),
        (161..=200, bids(0.4)),
        (201..=201, bids(0.8)),
    ] {
        for column in columns {
            assert!(reaches(&down.columns[column], wanted), "column {column}");
        }
    }

    // A plot longer than a view holds is cut, each tile with the column
    // before its first and the one after its last.
    let long = build(&single(), &[steps()], (1_000.0, 122.0), None);
    assert_eq!(long.columns.len(), 1_002);
    assert_eq!(long.tiles.len(), 5);
    let mut end = 0.0;
    for tile in &long.tiles {
        assert_eq!(tile.offset, end);
        assert_eq!(tile.first, tile.offset as usize);
        assert_eq!(tile.count, tile.length as usize + 2);
        assert!(tile.count <= COLUMN_CAPACITY);
        end += tile.length;
    }
    assert_eq!(end, 1_000.0);
}

/// Two books a unit either side of 100.5, looked at twice as far: the
/// first 1 deep a side, the second 1 deep in bids and 3 in asks.
fn two_books(blend: Blend) -> Frame {
    let books = [
        book(&[(100.0, 1.0)], &[(101.0, 1.0)]),
        book(&[(100.0, 1.0)], &[(101.0, 3.0)]),
    ];
    let config = Config { blend, ..config() };
    build(&config, &books, (100.0, 122.0), Some(0.01))
}

#[test]
fn two_books_are_drawn_over_one_another_stacked_or_combined() {
    // The bids are left of column 26, the asks right of column 75.
    let (bid, ask) = (11, 91);
    let overlaid = two_books(Blend::Overlaid);
    assert_eq!(overlaid.top, 4.0);
    assert!(reaches(&overlaid.columns[bid], [0.25, 0.0, 0.25, 0.0]));
    assert!(reaches(&overlaid.columns[ask], [0.0, 0.25, 0.0, 0.75]));
    assert!(reaches(&overlaid.columns[50], [0.0; 4]));
    // Stacked, the second book's series are both books' depth.
    let stacked = two_books(Blend::Stacked);
    assert_eq!(stacked.top, 5.0);
    assert!(reaches(&stacked.columns[bid], [0.2, 0.0, 0.4, 0.0]));
    assert!(reaches(&stacked.columns[ask], [0.0, 0.2, 0.0, 0.8]));
    // Combined, that sum is all there is, as one book's.
    let combined = two_books(Blend::Combined);
    assert_eq!(combined.top, 5.0);
    assert!(reaches(&combined.columns[bid], [0.4, 0.0, 0.0, 0.0]));
    assert!(reaches(&combined.columns[ask], [0.0, 0.8, 0.0, 0.0]));

    // Each series in its book's colors; the sum in the first book's.
    let tints = |config: &Config| -> Vec<Option<(Pair, Side)>> {
        (0..4).map(|series| config.tint(series)).collect()
    };
    let (first, second) = (Pair::Primary, Pair::Secondary);
    let apart = [
        Some((first, Side::Bid)),
        Some((first, Side::Ask)),
        Some((second, Side::Bid)),
        Some((second, Side::Ask)),
    ];
    assert_eq!(tints(&overlaid.config), apart);
    assert_eq!(tints(&stacked.config), apart);
    assert_eq!(tints(&combined.config), [apart[0], apart[1], None, None]);
    assert_eq!(tints(&single()), [apart[0], apart[1], None, None]);
    assert!(stacked.config.stacked() && !stacked.config.combined());
    // One book has nothing to be stacked on or summed with.
    let alone = Config {
        blend: Blend::Stacked,
        ..single()
    };
    assert!(!alone.stacked() && !alone.combined());
    // Both books count in BTC; a second in another asset has no shared
    // one.
    assert_eq!(overlaid.config.asset(), Some("BTC"));
    let mixed = Config {
        layers: vec![
            layer(Pair::Primary, bybit()),
            layer(
                Pair::Secondary,
                MarketSelection {
                    base: "ETH".into(),
                    ..binance()
                },
            ),
        ],
        ..config()
    };
    assert_eq!(mixed.asset(), None);
}

/// The shader's layer for the one tile of `frame`.
fn shader_layer(frame: &Frame) -> ShaderLayer {
    let program = shader::program().expect("the depth chart's shader compiles");
    let uniforms = shader::uniforms(frame, &frame.tiles[0], &palette());
    let mut warnings = Vec::new();
    let layer = ShaderLayer::new(program, &[("d".into(), uniforms)], &mut warnings);
    assert_eq!(warnings, Vec::<String>::new());
    layer
}

#[test]
fn the_shader_compiles_and_takes_a_tile_in_one_array() {
    let program = shader::program().expect("the depth chart's shader compiles");
    assert_eq!(program.uniform_size(), 4_096);
    assert!(
        !program.animated(),
        "it draws only when its uniforms change"
    );
    assert!(program.inputs().is_empty());

    let vectors = |frame: &Frame| -> Vec<[f32; 4]> {
        let layer = shader_layer(frame);
        let (vectors, _) = layer.uniforms.as_chunks::<4>();
        vectors
            .iter()
            .map(|vector| vector.map(f32::from_bits))
            .collect()
    };
    let across = vectors(&stepped(BookLayout::Horizontal));
    // The view: the first column starts a point before the tile, 202
    // columns, prices across the view, nothing stacked.
    assert_eq!(across[0], [-1.0, 202.0, 0.0, 0.0]);
    // The primary's bids and asks; no second pair.
    assert_eq!(across[1], [0.0, 1.0, 0.0, 1.0]);
    assert_eq!(across[2], [1.0, 0.0, 0.0, 1.0]);
    assert_eq!((across[3], across[4]), ([0.0; 4], [0.0; 4]));
    // The column before the plot, then the plot's first.
    assert!(reaches(&across[5], [0.8, 0.0, 0.0, 0.0]));
    assert!(reaches(&across[6], [0.4, 0.0, 0.0, 0.0]));

    assert_eq!(vectors(&stepped(BookLayout::Vertical))[0][2], 1.0);
    let stacked = vectors(&two_books(Blend::Stacked));
    assert_eq!(stacked[0][3], 1.0);
    assert_eq!(stacked[3], [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(stacked[4], [1.0, 1.0, 0.0, 1.0]);
    // The second pair alone draws in its own colors.
    let config = Config {
        layers: vec![layer(Pair::Secondary, binance())],
        ..single()
    };
    let alone = vectors(&build(&config, &[steps()], (200.0, 122.0), None));
    assert_eq!(alone[1], [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(alone[2], [1.0, 1.0, 0.0, 1.0]);
}

/// Whether `pixel` is `tint` (red, green and blue, each on or off) at an
/// area's opacity, neither clear nor a line.
fn filled(pixel: [u8; 4], tint: [bool; 3]) -> bool {
    let alpha = pixel[3];
    (20..=110).contains(&alpha)
        && tint
            .iter()
            .zip(pixel)
            .all(|(on, channel)| channel == if *on { alpha } else { 0 })
}

const BLUE: [u8; 4] = [0, 0, 255, 255];
const YELLOW: [u8; 4] = [255, 255, 0, 255];

#[test]
fn the_gpu_draws_areas_and_lines_where_the_frame_puts_them() {
    let frame = stepped(BookLayout::Horizontal);
    let Some(picture) = drawn(shader_layer(&frame), 200.0, 100.0, 1.0) else {
        eprintln!("no GPU here: the depth chart's pixels were not checked");
        return;
    };
    assert_eq!((picture.width, picture.height), (200, 100));
    let (green, red) = ([false, true, false], [true, false, false]);
    // The bids through 99 reach 40 pixels up: their line on the pixel
    // under that, their area below it, nothing above.
    assert_eq!(pixel(&picture, 20, 50), CLEAR);
    assert_eq!(pixel(&picture, 20, 60), GREEN);
    assert!(filled(pixel(&picture, 20, 80), green));
    assert!(filled(pixel(&picture, 20, 99), green));
    // An area is fainter toward the baseline.
    assert!(pixel(&picture, 20, 65)[3] > pixel(&picture, 20, 95)[3]);
    // The step down to the bids through 100 is a line on the taller
    // column's last pixel, from one reach to the other.
    assert_eq!(pixel(&picture, 39, 70), GREEN);
    assert!(filled(pixel(&picture, 38, 70), green));
    assert_eq!(pixel(&picture, 60, 80), GREEN);
    assert_eq!(pixel(&picture, 60, 70), CLEAR);
    // The best bid's edge goes down to the baseline; the spread is clear.
    assert_eq!(pixel(&picture, 79, 90), GREEN);
    assert_eq!(pixel(&picture, 79, 99), GREEN);
    assert_eq!(pixel(&picture, 100, 99), CLEAR);
    assert_eq!(pixel(&picture, 118, 90), CLEAR);
    // The asks: up from the baseline at 101, a step up at 102.
    assert_eq!(pixel(&picture, 120, 90), RED);
    assert_eq!(pixel(&picture, 140, 60), RED);
    assert!(filled(pixel(&picture, 140, 80), red));
    assert_eq!(pixel(&picture, 160, 40), RED);
    assert_eq!(pixel(&picture, 180, 20), RED);
    assert_eq!(pixel(&picture, 180, 10), CLEAR);
    // The curves go on past the plot: no line closes them at its edges.
    assert!(filled(pixel(&picture, 0, 80), green));
    assert!(filled(pixel(&picture, 199, 50), red));

    // At two pixels a point a line is three pixels, about the same reach.
    let dense = drawn(shader_layer(&frame), 200.0, 100.0, 2.0).expect("the GPU answered before");
    assert_eq!((dense.width, dense.height), (400, 200));
    assert_eq!(pixel(&dense, 40, 117), CLEAR);
    for y in 119..=121 {
        assert_eq!(pixel(&dense, 40, y), GREEN, "{y}");
    }
    assert!(filled(pixel(&dense, 40, 124), green));
    assert_eq!(pixel(&dense, 360, 40), RED);

    // Down the pane depth grows to the right: asks at the top, bids at
    // the bottom.
    let down = drawn(
        shader_layer(&stepped(BookLayout::Vertical)),
        100.0,
        200.0,
        1.0,
    )
    .expect("the GPU answered before");
    assert_eq!(pixel(&down, 79, 20), RED);
    assert_eq!(pixel(&down, 90, 20), CLEAR);
    assert!(filled(pixel(&down, 40, 20), red));
    assert_eq!(pixel(&down, 60, 39), RED);
    assert_eq!(pixel(&down, 39, 60), RED);
    assert_eq!(pixel(&down, 50, 100), CLEAR);
    assert_eq!(pixel(&down, 19, 140), GREEN);
    assert_eq!(pixel(&down, 39, 180), GREEN);
    assert!(filled(pixel(&down, 10, 180), green));

    // Stacked, the second book's band starts where the first's ends, and
    // its line is their sum's.
    let stacked = drawn(shader_layer(&two_books(Blend::Stacked)), 100.0, 100.0, 1.0)
        .expect("the GPU answered before");
    let (blue, yellow) = ([false, false, true], [true, true, false]);
    assert_eq!(pixel(&stacked, 10, 80), GREEN);
    assert!(filled(pixel(&stacked, 10, 90), green));
    assert!(filled(pixel(&stacked, 10, 70), blue));
    assert_eq!(pixel(&stacked, 10, 60), BLUE);
    assert_eq!(pixel(&stacked, 10, 50), CLEAR);
    assert_eq!(pixel(&stacked, 90, 80), RED);
    assert!(filled(pixel(&stacked, 90, 50), yellow));
    assert_eq!(pixel(&stacked, 90, 20), YELLOW);
    // Combined, one area a side reaches as far.
    let combined = drawn(shader_layer(&two_books(Blend::Combined)), 100.0, 100.0, 1.0)
        .expect("the GPU answered before");
    assert_eq!(pixel(&combined, 10, 60), GREEN);
    assert!(filled(pixel(&combined, 10, 80), green));
    assert_eq!(pixel(&combined, 90, 20), RED);
    assert!(filled(pixel(&combined, 90, 80), red));
}

/// The overlay of `frame`, with the pointer where `pointer` says.
fn overlaid(frame: &Frame, pointer: Option<(f64, f64)>) -> (Record, Vec<Logo>) {
    let drawn = overlay::draw(frame, pointer, &colors(), false);
    (Record::of(&drawn.painter.finish()), drawn.logos)
}

/// `color` at `alpha` of its own opacity.
fn faded(color: Hsla, alpha: f32) -> Hsla {
    Hsla {
        a: color.a * alpha,
        ..color
    }
}

#[test]
fn depths_are_written_in_a_few_digits() {
    for (depth, text) in [
        (0.0, "0"),
        (0.004, "0.004"),
        (0.0625, "0.0625"),
        (0.375, "0.375"),
        (1.0, "1"),
        (2.5, "2.5"),
        (12.345, "12.3"),
        (250.0, "250"),
        (499.6, "500"),
        (19_300.0, "19.3k"),
        (1_250_000.0, "1.25M"),
        (3_000_000_000.0, "3B"),
        (f64::NAN, "--"),
    ] {
        assert_eq!(depth_text(depth), text, "{depth}");
    }
}

#[test]
fn the_overlay_names_prices_depths_and_the_mid_price() {
    let across = stepped(BookLayout::Horizontal);
    let (record, logos) = overlaid(&across, None);
    assert!(logos.is_empty());
    // The marks' prices in the strip, each under its mark where it fits
    // whole: the middle's here, the others are too near the edges.
    let name = record.find("100.5");
    assert_eq!((name.x, name.y), (100.0, 111.5));
    assert!(same(name.color, colors().dim));
    assert_eq!((record.count("98.3"), record.count("102.7")), (0, 0));
    // A rule's depth under its rule at the plot's right edge, and the
    // asset on the one at its far edge.
    assert_eq!(record.words()[1..6], ["1", "2", "3", "4", "5 BTC"]);
    assert_eq!((record.find("1").x, record.find("1").y), (194.0, 88.0));
    assert_eq!(record.find("5 BTC").y, 8.0);
    // Rules: five depths and three marks, fainter than the strip's edge.
    assert_eq!(record.ruled(faded(colors().grid, 0.5)).len(), 8);
    assert_eq!(record.ruled(colors().grid).len(), 1);
    // The mid price: a dashed line at the plot's middle, named above it.
    let mid = record.ruled(colors().accent);
    assert_eq!(mid.len(), 1);
    assert!(mid[0].dashed && mid[0].from.0 == 100.5);
    let label = record.find("100.500");
    assert_eq!((label.x, label.y), (79.0, 9.0));
    assert!(same(label.color, colors().accent));

    // Down the pane the prices are in their column, each at its mark,
    // and the depths along the bottom, as many as fit.
    let down = stepped(BookLayout::Vertical);
    let (record, _) = overlaid(&down, None);
    for (price, y) in [
        ("102.7", 12.0),
        ("101.6", 56.0),
        ("100.5", 100.0),
        ("99.4", 144.0),
        ("98.3", 188.0),
    ] {
        let name = record.find(price);
        assert_eq!((name.x, name.y), (6.0, y), "{price}");
        assert!(same(name.color, colors().text));
    }
    assert_eq!(record.count("5 BTC"), 1);
    assert_eq!(record.find("5 BTC").x, 162.0);
    assert_eq!(record.find("5 BTC").y, 192.0);
    assert_eq!((record.count("3"), record.count("4")), (1, 0));
    let label = record.find("100.500");
    assert_eq!((label.x, label.y), (118.0, 91.0));

    // The settings can leave the mid price out.
    let plain = Config {
        mid_price: false,
        ..single()
    };
    let (plain, _) = overlaid(&build(&plain, &[steps()], (200.0, 122.0), None), None);
    assert_eq!(plain.count("100.500"), 0);
    assert!(plain.ruled(colors().accent).is_empty());
    // A warning is written above the plot, which starts under it.
    let warned = Config {
        warning: Some("Pair 2 duplicates Primary and is not rendered."),
        ..single()
    };
    let warned = build(&warned, &[steps()], (200.0, 142.0), None);
    assert_eq!((warned.plot_top, warned.plot_bottom), (20.0, 120.0));
    let (warned, _) = overlaid(&warned, None);
    assert_eq!(
        warned
            .find("Pair 2 duplicates Primary and is not rendered.")
            .y,
        10.0
    );

    // Nothing has arrived: the plot says it waits, or cannot.
    let mut waiting = build(&single(), &[None], (200.0, 122.0), None);
    assert_eq!(overlaid(&waiting, None).0.words(), [WAITING]);
    waiting.failed = vec![true];
    assert_eq!(overlaid(&waiting, None).0.words(), [UNAVAILABLE]);
}

#[test]
fn two_books_mid_prices_are_ruled_and_named_apart() {
    let books = [
        book(&[(100.0, 1.0)], &[(101.0, 1.0)]),
        book(&[(100.0, 1.0)], &[(101.0, 3.0)]),
    ];
    let frame = build(&config(), &books, (300.0, 122.0), Some(0.01));
    let (record, logos) = overlaid(&frame, None);
    // Both at 100.5: a line each in its book's color, the second's label
    // under the first's, each after its exchange's logo.
    assert_eq!(record.count("100.500"), 2);
    assert_eq!(record.ruled(colors().accent).len(), 1);
    assert_eq!(record.ruled(colors().amber).len(), 1);
    let labels: Vec<_> = record
        .texts
        .iter()
        .filter(|text| text.text == "100.500")
        .collect();
    assert!(same(labels[0].color, colors().accent) && same(labels[1].color, colors().amber));
    assert_eq!((labels[0].y, labels[1].y), (9.0, 25.0));
    assert_eq!(logos.len(), 2);
    assert_eq!(
        (logos[0].exchange.as_str(), logos[1].exchange.as_str()),
        ("bybit", "binance")
    );
    assert_eq!((logos[0].y, logos[1].y), (5.0, 21.0));
    assert_eq!(labels[0].x, logos[0].x + 11.0);

    // Combined they are one book, with one mid price and no logo.
    let combined = Config {
        blend: Blend::Combined,
        ..config()
    };
    let (record, logos) = overlaid(&build(&combined, &books, (300.0, 122.0), Some(0.01)), None);
    assert_eq!(record.count("100.500"), 1);
    assert_eq!(record.ruled(colors().accent).len(), 1);
    assert!(record.ruled(colors().amber).is_empty() && logos.is_empty());
}

#[test]
fn without_a_shader_the_overlay_draws_the_curves() {
    let close = |rect: &[f32; 4], wanted: [f32; 4]| {
        rect.iter()
            .zip(wanted)
            .all(|(edge, wanted)| (edge - wanted).abs() < 1e-3)
    };
    let areas = |frame: &Frame, color: Hsla| -> Vec<[f32; 4]> {
        let drawn = overlay::draw(frame, None, &colors(), true);
        let record = Record::of(&drawn.painter.finish());
        let boxes = record.boxes.iter();
        boxes
            .filter(|(_, fill)| same(*fill, faded(color, FILL)))
            .map(|(rect, _)| *rect)
            .collect()
    };
    // A rectangle per run of columns a series reaches as far in: left,
    // top, width and height.
    let across = stepped(BookLayout::Horizontal);
    let bids = areas(&across, colors().bid);
    assert_eq!(bids.len(), 2);
    assert!(close(&bids[0], [0.0, 60.0, 40.0, 40.0]) && close(&bids[1], [40.0, 80.0, 40.0, 20.0]));
    let asks = areas(&across, colors().ask);
    assert_eq!(asks.len(), 2);
    assert!(close(&asks[0], [120.0, 60.0, 40.0, 40.0]));
    assert!(close(&asks[1], [160.0, 20.0, 40.0, 80.0]));
    // Down the pane they grow right from the price column.
    let down = stepped(BookLayout::Vertical);
    let asks = areas(&down, colors().ask);
    assert!(close(&asks[0], [66.0, 0.0, 80.0, 40.0]) && close(&asks[1], [66.0, 40.0, 40.0, 40.0]));
    let bids = areas(&down, colors().bid);
    assert!(close(&bids[0], [66.0, 120.0, 20.0, 40.0]));
    assert!(close(&bids[1], [66.0, 160.0, 40.0, 40.0]));
    // Stacked, the second book's band starts where the first's ends.
    let stacked = two_books(Blend::Stacked);
    let band = areas(&stacked, colors().accent);
    assert_eq!(band.len(), 1);
    assert!(close(&band[0], [0.0, 60.0, 25.0, 20.0]), "{band:?}");
    // Each run's line, and one up the edge between two: the bids' two
    // runs, the step between them and the edge at the best bid.
    let drawn = overlay::draw(&across, None, &colors(), true);
    let record = Record::of(&drawn.painter.finish());
    assert_eq!(record.ruled(colors().bid).len(), 4);
    assert_eq!(record.ruled(colors().ask).len(), 4);
    // With a shader, it leaves them to it.
    let (plain, _) = overlaid(&across, None);
    assert!(plain.boxes.is_empty() && plain.ruled(colors().bid).is_empty());
}

#[test]
fn the_pointers_box_counts_what_rests_within_its_distance() {
    // 1.75 from the center: the bids down to 98.75, the asks up to
    // 102.25.
    let frame = stepped(BookLayout::Horizontal);
    let (record, _) = overlaid(&frame, Some((30.0, 50.0)));
    for text in [
        "From mid", "±1.74%", "Bids to", "98.750", "Asks to", "102.250", "Bybit", "BTC/USDT",
        "2 BTC", "199 USDT", "4 BTC", "406 USDT",
    ] {
        assert_eq!(record.count(text), 1, "{text}");
    }
    assert_eq!((record.count("Amount"), record.count("Value")), (2, 2));
    assert_eq!(record.count("Total"), 0);
    assert!(same(record.find("Bids to").color, colors().bid));
    assert!(same(record.find("Asks to").color, colors().ask));
    assert!(same(record.find("Bybit").color, colors().accent));
    // The same distance the other side of the center is the same box.
    let (mirrored, _) = overlaid(&frame, Some((170.0, 50.0)));
    for text in ["±1.74%", "98.750", "102.250", "2 BTC", "4 BTC"] {
        assert_eq!(mirrored.count(text), 1, "{text}");
    }
    // The pointer's line, its mirror, and the band between them.
    let lines = record.ruled(faded(colors().text, 0.7));
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].from, (30.5, 0.0));
    let mirror = record.ruled(faded(colors().text, 0.35));
    assert_eq!((mirror.len(), mirror[0].from), (1, (170.5, 0.0)));
    let band = record
        .boxes
        .iter()
        .find(|(_, fill)| same(*fill, faded(colors().text, 0.05)));
    assert_eq!(band.map(|(rect, _)| *rect), Some([30.0, 0.0, 140.0, 100.0]));

    // Off the plot (over the strip of prices) there is no box.
    let (off, _) = overlaid(&frame, Some((30.0, 110.0)));
    assert_eq!(off.count("From mid"), 0);
    assert!(off.ruled(faded(colors().text, 0.7)).is_empty());
    // Nor before a book has arrived.
    let waiting = build(&single(), &[None], (200.0, 122.0), None);
    assert_eq!(overlaid(&waiting, Some((30.0, 50.0))).0.words(), [WAITING]);

    // Two books each say what they hold, then both together; one that
    // has not arrived has nothing to count.
    let books = [steps(), book(&[(100.2, 3.0)], &[(100.8, 5.0)])];
    let zoom = Some(2.5 / 100.5);
    let both = build(&config(), &books, (600.0, 322.0), zoom);
    let (record, _) = overlaid(&both, Some((90.0, 50.0)));
    for text in [
        "Bybit", "Binance", "Total", "2 BTC", "3 BTC", "5 BTC", "199 USDT", "301 USDT", "500 USDT",
        "4 BTC", "9 BTC", "406 USDT", "504 USDT", "910 USDT",
    ] {
        assert!(record.count(text) >= 1, "{text}: {:?}", record.words());
    }
    assert_eq!(record.count("BTC/USDT"), 2);
    assert!(same(record.find("Binance").color, colors().amber));
    let late = build(&config(), &[steps(), None], (600.0, 322.0), zoom);
    let (record, _) = overlaid(&late, Some((90.0, 50.0)));
    assert_eq!(record.count("—"), 4);
    assert!(record.count("2 BTC") >= 2 && record.count("Total") == 1);
}

/// A workspace of one depth chart, with its data services as the shell
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
    layout::spawn(&mut saved, WidgetKind::DepthChart);
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
fn a_depth_chart_shows_its_book_and_follows_its_settings() {
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
    let mut config = EngineConfig::new(Root::of::<Harness>());
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
    let count = {
        let written = written.clone();
        move |text: &str| -> usize {
            let written = written.lock().unwrap();
            written.iter().filter(|word| *word == text).count()
        }
    };
    let writes = |text: &str| count(text) > 0;
    let newest = |word: usize| {
        layers
            .lock()
            .unwrap()
            .last()
            .map(|words: &Vec<f32>| words[word])
    };

    // Bybit's BTC/USDT by default, 200 levels a side: the feed a default
    // order book of it shares.
    app.wait_for("BTC/USDT").unwrap();
    app.wait_until("the book", WAIT, |_| writes("100.250"))
        .unwrap();
    let first = asked();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0]["limit"], 200);
    assert_eq!(first[0]["symbol"], "BTC/USDT:USDT");
    // 6 BTC rest through the bids, 4 through the asks: rules every 2, to
    // 8. The mid price on its line.
    for text in ["2", "4", "6", "8 BTC"] {
        assert!(writes(text), "{text}");
    }
    assert!(!writes(WAITING) && !writes("From mid"));
    // Tiles of curves, prices running across them, nothing stacked.
    assert_eq!((newest(2), newest(3)), (Some(0.0), Some(0.0)));

    // The top bar's selects: a grouping merges the asks into one step,
    // and the window goes as far as its groups.
    app.press("Group").unwrap();
    app.press("10").unwrap();
    app.wait_until("grouped steps", WAIT, |_| writes("5 BTC"))
        .unwrap();
    assert!(!writes("8 BTC"));
    // Another depth is another book.
    app.press("Depth").unwrap();
    app.press("50").unwrap();
    app.wait_until("the shallower book", WAIT, |_| asked().len() == 2)
        .unwrap();
    assert_eq!(asked()[1]["limit"], 50);

    // The pointer's box says what rests within its distance of the mid:
    // at the plot's middle, none, between the mid price and itself.
    app.hover("id:depth-chart-plot").unwrap();
    app.wait_until("the pointer's box", WAIT, |_| writes("From mid"))
        .unwrap();
    assert!(writes("Bids to") && writes("Asks to") && writes("Bybit"));
    assert!(writes("±0.00%"));
    assert_eq!(count("100.250"), 3);

    // The settings panel: depth as a value, the prices down the pane,
    // and no mid price.
    app.press("Widget settings").unwrap();
    app.wait_for("id:depth-chart-settings").unwrap();
    app.press("Depth in").unwrap();
    app.press("Value").unwrap();
    app.wait_until("depth in USDT", WAIT, |_| writes("500 USDT"))
        .unwrap();
    app.press("Layout").unwrap();
    app.press("Vertical").unwrap();
    app.wait_until("prices down the pane", WAIT, |_| newest(2) == Some(1.0))
        .unwrap();
    assert_eq!(count("100.250"), 3);
    // Without its label the mid price is only the box's two prices.
    app.press("Show mid price").unwrap();
    app.wait_until("no mid price", WAIT, |_| count("100.250") == 2)
        .unwrap();

    // The top bar's + opens the market picker for a second pair.
    app.press("Track another pair").unwrap();
    app.wait_for("Search for symbol").unwrap();
    app.press_key("Escape").unwrap();
    app.wait_for_gone("Search for symbol", WAIT).unwrap();
    assert!(app.shows("BTC/USDT"));
}
