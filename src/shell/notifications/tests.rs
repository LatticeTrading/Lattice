//! The notifications, headless: the bell and the popups over a center
//! that buttons post to ([`Board`]), and the connected account's trades
//! told over the trading widgets' bench ([`Desk`]).

use std::{
    rc::Rc,
    sync::Arc,
    time::{Duration, SystemTime},
};

use nsh_core::{
    platform::{Idiom, Os},
    time::Clock as EngineClock,
};
use nsh_engine::EngineConfig;
use nsh_testing::{Headless, MemoryStorage, WAIT};

use super::*;
use crate::{
    data::DataServices,
    layout,
    notify::{KEPT, Notice, SHOWN},
    shell::everything_bar,
    storage::StoredWorkspace,
    theme::ActiveTheme,
    widgets::{
        WidgetKind,
        common::trade_bench::{self, Bench},
        picker::Picker,
    },
};

const BELL: &str = "id:notifications";
const LIST: &str = "id:notifications-panel";
const POPUPS: &str = "id:notification-popups";
/// What the bell is called while one notification is unread.
const ONE_UNREAD: &str = "label:Notifications: 1 unread";
/// The window the tests run in.
const WINDOW: (f32, f32) = (1280.0, 800.0);

/// The bar and the popups over a center of their own, with what a test
/// has happen: an order fills, a price is crossed, and notes that are
/// numbered as they arrive.
#[component]
fn Board() -> impl IntoElement {
    let center = Notifications::provide();
    let clock = Clock::restore();
    let mut saved = StoredWorkspace::default();
    layout::normalize(&mut saved);
    let workspace = signal(saved);
    let notes = signal(0_u32);
    let happens = |label: &'static str, notice: Rc<dyn Fn() -> Notice>| {
        view()
            .role(AccessibilityRole::Button)
            .label(label)
            .on_press(move || center.post(notice()))
            .child(text(label))
    };
    view()
        .size_full()
        .flex_col()
        .child(everything_bar::bar(
            workspace,
            ActiveTheme::restore(),
            clock,
            Picker::new(),
        ))
        .child(happens(
            "an order fills",
            Rc::new(|| {
                Notice::new(Level::Success, "Filled: Open Long")
                    .body("Bought 0.01 BTC at 61000.0.")
                    .origin("Hyperliquid")
            }),
        ))
        .child(happens(
            "a price is crossed",
            Rc::new(|| Notice::new(Level::Warning, "BTC crossed 62,000")),
        ))
        .child(happens(
            "a note arrives",
            Rc::new(move || {
                notes.update(|notes| *notes += 1);
                Notice::new(Level::Info, format!("Note {}", notes.get_untracked()))
            }),
        ))
        .child(popups(center, clock))
}

fn start() -> Headless {
    start_with(MemoryStorage::new([]))
}

/// The board on a desktop, on a clock the test moves.
fn start_with(storage: Arc<MemoryStorage>) -> Headless {
    let mut config = EngineConfig::new(nsh_testing::windowed_root::<Board>());
    config.viewport = Size::new(px(WINDOW.0), px(WINDOW.1));
    config.storage = Some(storage);
    config.clock = EngineClock::manual(SystemTime::now());
    config.environment.os = Os::Windows;
    config.environment.idiom = Idiom::Desktop;
    let mut app = Headless::start(config).unwrap();
    app.wait_for(BELL).unwrap();
    app
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

/// The left, top, right and bottom edges of `bounds`.
fn edges(bounds: Bounds<Points>) -> [f32; 4] {
    let (x, y) = (bounds.origin.x.0, bounds.origin.y.0);
    [x, y, x + bounds.size.width.0, y + bounds.size.height.0]
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.5
}

/// Longer than a popup shows.
fn a_while() -> Duration {
    POPUP_FOR + Duration::from_secs(1)
}

#[test]
fn a_notification_pops_up_in_the_corner_joins_the_bells_list_and_goes_by_itself() {
    let mut app = start();
    assert!(!app.screen().matches(POPUPS).unwrap());
    assert!(!app.screen().matches(ONE_UNREAD).unwrap());

    // It pops up in the bottom right corner, as wide as popups are, and the
    // bell says one is unread.
    app.press("an order fills").unwrap();
    app.wait_for(POPUPS).unwrap();
    for said in [
        "Filled: Open Long",
        "Bought 0.01 BTC at 61000.0.",
        "Hyperliquid",
    ] {
        assert!(app.shows(said), "{said}");
    }
    let [left, _, right, bottom] = edges(placed(&app, "notification-popups"));
    assert!(near(right, WINDOW.0 - EDGE), "{right}");
    assert!(near(bottom, WINDOW.1 - EDGE), "{bottom}");
    assert!(near(right - left, POPUP_WIDTH), "{}", right - left);
    app.wait_for(ONE_UNREAD).unwrap();
    // Its × is a cell of its row: as tall as it, at its far edge.
    let [_, row_top, row_right, row_bottom] = edges(placed(&app, "notification-popup-1"));
    let [_, top, x_right, x_bottom] = edges(placed(&app, "notification-dismiss-1"));
    assert!(near(top, row_top) && near(x_bottom, row_bottom) && near(x_right, row_right));

    // It goes by itself, and is still unread.
    app.run_for(a_while()).unwrap();
    app.wait_for_gone(POPUPS, WAIT).unwrap();
    assert!(app.screen().matches(ONE_UNREAD).unwrap());

    // The bell's list keeps it; opening the list is reading it, and so is
    // what arrives while it is open.
    app.press(BELL).unwrap();
    app.wait_for(LIST).unwrap();
    app.wait_for_gone(ONE_UNREAD, WAIT).unwrap();
    assert!(app.shows("Filled: Open Long") && app.shows("Bought 0.01 BTC at 61000.0."));
    assert!(!app.shows("No notifications yet."));
    app.press("a price is crossed").unwrap();
    app.wait_for(&format!("{LIST} >> text:BTC crossed 62,000"))
        .unwrap();
    assert!(!app.screen().matches(ONE_UNREAD).unwrap());

    // Cleared, nothing is listed and nothing shows.
    app.press("Clear all notifications").unwrap();
    app.wait_for("No notifications yet.").unwrap();
    assert!(!app.shows("Filled: Open Long") && !app.shows("BTC crossed 62,000"));
    assert!(!app.screen().matches(POPUPS).unwrap());
    assert!(!app.screen().enabled("Clear all notifications").unwrap());

    // What arrives once the list is closed is unread again.
    app.press("Close Notifications").unwrap();
    app.wait_for_gone(LIST, WAIT).unwrap();
    app.press("a price is crossed").unwrap();
    app.wait_for(ONE_UNREAD).unwrap();
}

#[test]
fn the_bell_is_the_cell_after_the_connection_status() {
    let app = start();
    let [_, top, connection_right, bottom] = edges(placed(&app, "connection-status"));
    let [left, bell_top, right, bell_bottom] = edges(placed(&app, "notifications"));
    // It begins where the cell before it ends, as wide and as high as it.
    assert!(near(left, connection_right), "{left} {connection_right}");
    assert!(near(right - left, 40.0), "{}", right - left);
    assert!(near(bell_top, top) && near(bell_bottom, bottom));
}

#[test]
fn the_newest_popups_show_and_one_under_the_pointer_stays() {
    let mut app = start();
    for _ in 0..=SHOWN {
        app.press("a note arrives").unwrap();
    }
    // One more than show at once: the oldest gave its place.
    let newest = SHOWN + 1;
    app.wait_for(&format!("Note {newest}")).unwrap();
    assert!(!app.shows("Note 1") && app.shows("Note 2"));
    // In a bottom corner the newest is the lowest, nearest the corner.
    let top_of =
        |app: &Headless, id: usize| placed(app, &format!("notification-popup-{id}")).origin.y.0;
    assert!(top_of(&app, newest) > top_of(&app, newest - 1));
    assert!(top_of(&app, newest - 1) > top_of(&app, newest - 2));

    // The one the pointer rests on stays while the others go.
    app.hover("id:notification-popup-3").unwrap();
    app.run_for(a_while()).unwrap();
    app.wait_for_gone("id:notification-popup-2", WAIT).unwrap();
    assert!(app.shows("Note 3") && !app.shows(&format!("Note {newest}")));
    // Left, it goes after its time.
    app.hover("id:clock").unwrap();
    app.run_for(a_while()).unwrap();
    app.wait_for_gone(POPUPS, WAIT).unwrap();

    // × takes one down at once, and the list keeps every one of them.
    app.press("a note arrives").unwrap();
    let last = newest + 1;
    app.wait_for(&format!("id:notification-popup-{last}"))
        .unwrap();
    app.press(&format!("id:notification-dismiss-{last}"))
        .unwrap();
    app.wait_for_gone(POPUPS, WAIT).unwrap();
    app.press(BELL).unwrap();
    app.wait_for(LIST).unwrap();
    for note in 1..=last {
        assert!(app.shows(&format!("Note {note}")), "{note}");
    }
    assert!(last < KEPT);
}

#[test]
fn settings_place_the_popups_turn_them_off_and_remember_both() {
    let storage = MemoryStorage::new([]);
    let mut app = start_with(storage.clone());
    app.press("Application settings").unwrap();
    app.wait_for("id:settings").unwrap();
    assert!(app.shows("Bottom right"));

    // A corner picked shows one there, under the bar, which the list does
    // not keep.
    app.press("role:combobox[Popup corner]").unwrap();
    app.press("role:option[Top left]").unwrap();
    app.wait_for("Notifications show here").unwrap();
    let [left, top, ..] = edges(placed(&app, "notification-popups"));
    assert!(near(left, EDGE) && near(top, UNDER_BAR), "{left} {top}");
    assert!(!app.screen().matches(ONE_UNREAD).unwrap());
    // In a top corner the newest is the highest.
    app.press("a note arrives").unwrap();
    app.wait_for("Note 1").unwrap();
    let top_of = |id: &str| placed(&app, id).origin.y.0;
    assert!(top_of("notification-popup-2") < top_of("notification-popup-1"));

    // Turned off, those that show go and none pops up; the bell still
    // counts them.
    app.press("Notification popups off").unwrap();
    app.wait_for_gone(POPUPS, WAIT).unwrap();
    app.press("an order fills").unwrap();
    app.wait_for("label:Notifications: 2 unread").unwrap();
    assert!(!app.screen().matches(POPUPS).unwrap());

    // Both are saved: the next launch pops nothing up, and turned on again
    // they show in the corner chosen.
    drop(app);
    let mut app = start_with(storage);
    app.press("an order fills").unwrap();
    app.wait_for(ONE_UNREAD).unwrap();
    assert!(!app.screen().matches(POPUPS).unwrap());
    app.press("Application settings").unwrap();
    app.wait_for("id:settings").unwrap();
    assert!(app.shows("Top left"));
    app.press("Notification popups on").unwrap();
    app.wait_for("Notifications show here").unwrap();
    let [left, top, ..] = edges(placed(&app, "notification-popups"));
    assert!(near(left, EDGE) && near(top, UNDER_BAR), "{left} {top}");
}

/// The trading widgets' bench (a stand-in wallet over a stubbed
/// Hyperliquid) with the shell's notifications over it: the account's
/// trades are told, and popped up.
#[component]
fn Desk() -> impl IntoElement {
    let center = Notifications::provide();
    let bench = trade_bench::workspace(WidgetKind::TradePanel, None);
    fills::announce(DataServices::get().trading.clone(), center);
    view()
        .size_full()
        .flex_col()
        .child(bench)
        .child(popups(center, Clock::restore()))
}

#[test]
fn the_connected_account_s_trades_are_told_whatever_shows_them() {
    let Bench {
        mut app, exchange, ..
    } = trade_bench::start_live(nsh_testing::windowed_root::<Desk>());
    app.wait_for("the wallet connects").unwrap();
    assert!(exchange.subscribed().is_empty());

    // The account is followed from the moment its wallet connects, with
    // trading not enabled and no widget asking for it.
    app.press("the wallet connects").unwrap();
    let of_account = || {
        let subscribed = exchange.subscribed().into_iter();
        subscribed
            .filter(|topic| topic.get("user").is_some())
            .count()
    };
    app.wait_until("the account being followed", WAIT, |_| of_account() >= 3)
        .unwrap();
    assert!(app.shows("hyperliquid is Locked"));
    assert!(!app.screen().matches(POPUPS).unwrap());

    // A trade of the account's, made anywhere, pops up as the exchange
    // says it.
    exchange.sold(3001, "ETH", "1891.4", "2.0");
    app.wait_for(&format!("{POPUPS} >> text:Sold 2 ETH at 1891.40."))
        .unwrap();
    assert!(app.shows("Filled: Open Long") && app.shows("Hyperliquid"));
}
