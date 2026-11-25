use super::*;
use nsh_engine::EngineConfig;
use nsh_testing::{DragOptions, Headless};

const WIDTH: f32 = 1264.0;

fn saved_workspace() -> StoredWorkspace {
    let mut saved = StoredWorkspace::default();
    normalize(&mut saved);
    super::spawn(&mut saved, WidgetKind::MarketScreener);
    super::spawn(&mut saved, WidgetKind::MarketScreener);
    saved
}

#[test]
fn clearing_pages_deletes_them_and_empties_the_last_one() {
    let mut saved = saved_workspace();
    let first = saved.pages[0].id;
    add_page(&mut saved);
    let second = saved.active_page_id;
    add_page(&mut saved);
    let third = saved.active_page_id;
    // Clearing the active page selects the one before it.
    assert!(clear_page(&mut saved, third));
    assert_eq!(saved.active_page_id, second);
    // Clearing an inactive page keeps the selection.
    assert!(clear_page(&mut saved, first));
    assert_eq!(saved.active_page_id, second);
    assert_eq!(saved.pages.len(), 1);
    // The last page stays, emptied of its widgets.
    super::spawn(&mut saved, WidgetKind::MarketScreener);
    assert!(clear_page(&mut saved, second));
    assert_eq!(saved.pages.len(), 1);
    assert_eq!(saved.active_page_id, second);
    assert_eq!(saved.pages[0].grid, Grid::default());
    assert!(!clear_page(&mut saved, 999));
}

#[test]
fn layout_codes_round_trip_and_reject_invalid_data() {
    let saved = saved_workspace();
    let code = export_code(&saved).unwrap();
    assert!(code.starts_with("LATTICE1."));
    // The saved form round-trips; an imported tab is a new widget.
    assert_eq!(export_code(&import_code(&code).unwrap()).unwrap(), code);
    assert!(import_code("not-a-layout").is_err());
    assert!(import_code("LATTICE1.0").is_err());
}

/// The saved settings of `widget`'s tab, in each breakpoint's copy.
fn saved_settings(saved: &StoredWorkspace, widget: WidgetId) -> Vec<serde_json::Value> {
    saved
        .pages
        .iter()
        .flat_map(|page| page.grid.layouts.iter().flatten())
        .flat_map(|pane| &pane.tabs)
        .filter(|tab| tab.widget == widget)
        .filter_map(|tab| match &tab.settings {
            TabWidget::Known(settings) => Some(settings.to_value()),
            TabWidget::Unknown { .. } => None,
        })
        .collect()
}

#[test]
fn a_linked_market_goes_to_the_widgets_of_its_color_on_every_page() {
    let mut saved = StoredWorkspace::default();
    normalize(&mut saved);
    super::spawn(&mut saved, WidgetKind::Chart);
    super::spawn(&mut saved, WidgetKind::Chart);
    super::spawn(&mut saved, WidgetKind::MarketScreener);
    add_page(&mut saved);
    super::spawn(&mut saved, WidgetKind::Chart);
    let widget = |page: usize, pane: usize| saved.pages[page].grid.layouts[4][pane].tabs[0].widget;
    let (blue, red, screener, other_page) =
        (widget(0, 0), widget(0, 1), widget(0, 2), widget(1, 0));

    // Charts are linked to blue until told otherwise; a screener cannot be.
    assert_eq!(link_counts(&saved), [0, 0, 3, 0, 0]);
    assert!(set_link(&mut saved, red, Some(LinkColor::Red)));
    assert!(!set_link(&mut saved, screener, Some(LinkColor::Red)));
    assert_eq!(link_counts(&saved), [0, 1, 2, 0, 0]);

    // A market sent to blue is the market of both blue charts, on either
    // page, in every breakpoint's copy of their tabs, and of nothing else.
    let market = MarketSelection {
        exchange: "hyperliquid".into(),
        symbol: "BTC/USDC".into(),
        quote: "USDC".into(),
        request_symbol: "BTC/USDC:USDC".into(),
        ..MarketSelection::bybit_btc_usdt()
    };
    assert_eq!(link_market(&mut saved, LinkColor::Blue, &market), 2);
    for widget in [blue, other_page] {
        let copies = saved_settings(&saved, widget);
        assert_eq!(copies.len(), 5);
        for copy in copies {
            assert_eq!(copy["market"]["exchange"], "hyperliquid");
            assert_eq!(copy.get("linkColor"), None);
        }
    }
    for copy in saved_settings(&saved, red) {
        assert_eq!(copy, serde_json::json!({ "linkColor": "red" }));
    }
    assert_eq!(link_market(&mut saved, LinkColor::Green, &market), 0);

    // A chart linked to no color takes no market, and counts for none.
    assert!(set_link(&mut saved, other_page, None));
    assert_eq!(link_counts(&saved), [0, 1, 1, 0, 0]);
    assert_eq!(
        link_market(
            &mut saved,
            LinkColor::Blue,
            &MarketSelection::bybit_btc_usdt()
        ),
        1
    );
    assert_eq!(
        saved_settings(&saved, other_page)[0]["market"]["exchange"],
        "hyperliquid"
    );

    // The links and the markets are in the layout's code.
    let code = export_code(&saved).unwrap();
    let imported = import_code(&code).unwrap();
    assert_eq!(link_counts(&imported), [0, 1, 1, 0, 0]);
    assert_eq!(export_code(&imported).unwrap(), code);
}

fn gesture(saved: &StoredWorkspace, resize: bool) -> DragState {
    let before = active_grid(saved).layouts[4].clone();
    DragState {
        id: before[0].id,
        page_id: saved.active_page_id,
        breakpoint: 4,
        width: WIDTH,
        preview: before.clone(),
        before,
        resize,
    }
}

#[test]
fn preview_is_transient_and_commit_changes_only_its_breakpoint() {
    let mut saved = saved_workspace();
    let original = saved.clone();
    let mut drag = gesture(&saved, true);
    drag.update(0.0, 32.0);
    assert_eq!(saved, original);
    assert_eq!(drag.preview[0].rect.h, 32);
    assert_eq!(drag.preview[1].rect.y, 32);
    assert!(drag.commit(&mut saved, WIDTH));
    assert_eq!(active_grid(&saved).layouts[4][0].rect.h, 32);
    assert_eq!(
        active_grid(&saved).layouts[..4],
        active_grid(&original).layouts[..4]
    );
    let json = serde_json::to_string(&saved).unwrap();
    let restored: StoredWorkspace = serde_json::from_str(&json).unwrap();
    assert_eq!(serde_json::to_string(&restored).unwrap(), json);
}

#[test]
fn stale_gestures_cannot_overwrite_pages_tabs_or_removed_panes() {
    for change in [
        add_page as fn(&mut StoredWorkspace),
        |s| add_tab(s, 1, WidgetKind::MarketScreener),
        |s| remove_pane(s, 1),
        |s| super::spawn(s, WidgetKind::MarketScreener),
    ] {
        let mut saved = saved_workspace();
        let mut drag = gesture(&saved, true);
        drag.update(0.0, 64.0);
        change(&mut saved);
        let changed = saved.clone();
        assert!(!drag.commit(&mut saved, WIDTH));
        assert_eq!(saved, changed);
    }
    let mut saved = saved_workspace();
    let mut drag = gesture(&saved, false);
    drag.update(200.0, 16.0);
    let original = saved.clone();
    assert!(!drag.commit(&mut saved, WIDTH - 1.0));
    assert_eq!(saved, original);
}

#[test]
fn returning_to_start_restores_snapshot_without_a_commit() {
    let mut saved = saved_workspace();
    // An intentional gap must not compact from a click or sub-cell motion.
    active_grid_mut(&mut saved).layouts[4][1].rect.y = 40;
    for resize in [false, true] {
        let mut drag = gesture(&saved, resize);
        drag.update(1.0, 1.0);
        assert_eq!(drag.preview, drag.before);
        drag.update(0.0, 128.0);
        assert_ne!(drag.preview, drag.before);
        drag.update(0.0, 0.0);
        assert_eq!(drag.preview, drag.before);
        assert!(!drag.commit(&mut saved, WIDTH));
    }
}

/// Every page's panes and their tabs in the widest breakpoint, the active
/// tab starred: `Tabs 1: 1[1*] 2[2*]`.
fn tabs_line(saved: &StoredWorkspace) -> String {
    let pages = saved.pages.iter().map(|page| {
        let panes = page.grid.layouts[4].iter().map(|pane| {
            let tabs = pane.tabs.iter().map(|tab| {
                let active = if tab.id == pane.active_tab { "*" } else { "" };
                format!("{}{active}", tab.id)
            });
            format!(" {}[{}]", pane.id, tabs.collect::<Vec<_>>().join(" "))
        });
        format!("{}:{}", page.id, panes.collect::<String>())
    });
    format!("Tabs {}", pages.collect::<Vec<_>>().join(" / "))
}

#[component]
fn Harness() -> impl IntoElement {
    let saved = signal(saved_workspace());
    view()
        .size_full()
        .flex_col()
        .child(text(move || {
            let state = saved.get();
            let rect = active_grid(&state).layouts[4][0].rect;
            format!("Saved {} {} {} {}", rect.x, rect.y, rect.w, rect.h)
        }))
        .child(text(move || saved.with(tabs_line)))
        .child(view().h(px(24.0)).flex_none().child(
            ui::button("Add a page", ui::Tone::Normal).on_press(move || saved.update(add_page)),
        ))
        .child(workspace(saved, crate::widgets::picker::Picker::new()))
}

#[test]
fn responder_survives_live_updates_and_commits_on_release() {
    let mut app = Headless::start(EngineConfig {
        viewport: Size::new(px(1280.0), px(900.0)),
        ..EngineConfig::new(nsh_testing::windowed_root::<Harness>())
    })
    .unwrap();
    app.wait_for("Saved 0 0 24 30").unwrap();
    app.drag_and_hold(
        "id:pane-resize-1",
        point(px(0.0), px(64.0)),
        DragOptions::default(),
    )
    .unwrap();
    assert!(
        app.shows("Saved 0 0 24 30"),
        "preview must not change committed state"
    );
    app.drop_held().unwrap();
    app.wait_for("Saved 0 0 24 34").unwrap();
    // A second gesture also proves the first responder was released.
    app.drag(
        "id:pane-move-1",
        point(px(106.0), px(32.0)),
        DragOptions::default(),
    )
    .unwrap();
    app.wait_for("Saved 4 2 24 34").unwrap();
}

#[test]
fn a_dragged_tab_goes_to_another_pane_or_another_place_among_its_tabs() {
    let mut app = Headless::start(EngineConfig {
        viewport: Size::new(px(1280.0), px(900.0)),
        ..EngineConfig::new(nsh_testing::windowed_root::<Harness>())
    })
    .unwrap();
    app.wait_for("Tabs 1: 1[1*] 2[2*]").unwrap();
    let shown = |app: &Headless, selector: &str| app.screen().matches(selector).unwrap();
    let settle = |app: &mut Headless| app.wait(std::time::Duration::from_millis(200)).unwrap();
    // The second pane is 30 rows below the first, its tab under the first's.
    let below = 30.0 * ROW_STEP;
    let tab_width = span(&app, "pane-tab-1-1").1;

    // Held over the leading half of the other pane's tab, the tab would go
    // before it: its title follows the pointer and a bar marks the gap, and
    // nothing has moved yet.
    app.drag_and_hold(
        "id:pane-tab-1-1",
        point(px(-10.0), px(below)),
        DragOptions::default(),
    )
    .unwrap();
    app.wait_for("id:tab-drag-ghost").unwrap();
    app.wait_for("id:pane-drop-2-0").unwrap();
    assert!(app.shows("Tabs 1: 1[1*] 2[2*]"));
    // Dropped, it is that pane's active tab; the pane it left stays, empty.
    app.drop_held().unwrap();
    app.wait_for("Tabs 1: 1[] 2[1* 2]").unwrap();
    assert!(!shown(&app, "id:tab-drag-ghost"));
    assert!(!shown(&app, "id:pane-drop-2-0"));
    app.wait_for("id:pane-tab-2-1").unwrap();

    // An empty pane takes it anywhere on its box, and says so.
    app.drag_and_hold(
        "id:pane-tab-2-1",
        point(px(0.0), px(100.0 - below)),
        DragOptions::default(),
    )
    .unwrap();
    app.wait_for("Drop tab here").unwrap();
    app.drop_held().unwrap();
    app.wait_for("Tabs 1: 1[1*] 2[2*]").unwrap();
    assert!(!app.shows("Drop tab here"));

    // Past the middle of the last tab it goes after it.
    app.drag_and_hold(
        "id:pane-tab-1-1",
        point(px(10.0), px(below)),
        DragOptions::default(),
    )
    .unwrap();
    app.wait_for("id:pane-drop-2-1").unwrap();
    assert!(!shown(&app, "id:pane-drop-2-0"));
    app.drop_held().unwrap();
    app.wait_for("Tabs 1: 1[] 2[2 1*]").unwrap();

    // Along its own pane's tabs it changes place, and the active tab stays.
    app.drag(
        "id:pane-tab-2-1",
        point(px(-tab_width - 10.0), px(0.0)),
        DragOptions::default(),
    )
    .unwrap();
    app.wait_for("Tabs 1: 1[] 2[1* 2]").unwrap();
    // A press still shows a tab; a drag that ends on the tab it began on
    // does not, and moves nothing.
    app.press("id:pane-tab-2-2").unwrap();
    app.wait_for("Tabs 1: 1[] 2[1 2*]").unwrap();
    app.drag(
        "id:pane-tab-2-1",
        point(px(12.0), px(0.0)),
        DragOptions::default(),
    )
    .unwrap();
    settle(&mut app);
    assert!(app.shows("Tabs 1: 1[] 2[1 2*]"));
    // Dropped off every pane, it stays where it was.
    app.drag(
        "id:pane-tab-2-1",
        point(px(900.0), px(-200.0)),
        DragOptions::default(),
    )
    .unwrap();
    settle(&mut app);
    assert!(app.shows("Tabs 1: 1[] 2[1 2*]"));

    // Escape puts a tab in hand back.
    app.drag_and_hold(
        "id:pane-tab-2-1",
        point(px(0.0), px(100.0 - below)),
        DragOptions::default(),
    )
    .unwrap();
    app.wait_for("Drop tab here").unwrap();
    app.press_key("escape").unwrap();
    settle(&mut app);
    assert!(!shown(&app, "id:tab-drag-ghost"));
    assert!(!app.shows("Drop tab here"));
    app.drop_held().unwrap();
    settle(&mut app);
    assert!(app.shows("Tabs 1: 1[] 2[1 2*]"));

    // A tab in hand whose page goes is put back: lifting the pointer on the
    // page shown then moves nothing, there or where the tab is.
    app.drag_and_hold(
        "id:pane-tab-2-1",
        point(px(0.0), px(100.0 - below)),
        DragOptions::default(),
    )
    .unwrap();
    app.wait_for("Drop tab here").unwrap();
    app.press("Add a page").unwrap();
    app.wait_for("Tabs 1: 1[] 2[1 2*] / 2:").unwrap();
    settle(&mut app);
    assert!(!shown(&app, "id:tab-drag-ghost"));
    app.drop_held().unwrap();
    settle(&mut app);
    assert!(app.shows("Tabs 1: 1[] 2[1 2*] / 2:"));
}

/// Where a view with a test id is laid out: its top edge and its height.
fn rise(app: &Headless, test_id: &str) -> (f32, f32) {
    let node = app
        .screen()
        .layout_nodes()
        .into_iter()
        .find(|node| node.test_id.as_deref() == Some(test_id))
        .unwrap_or_else(|| panic!("no view `{test_id}`"));
    (node.bounds.origin.y.0, node.bounds.size.height.0)
}

#[test]
fn a_tab_held_at_the_grids_edge_scrolls_it_to_the_panes_out_of_view() {
    let mut app = Headless::start(EngineConfig {
        viewport: Size::new(px(1280.0), px(900.0)),
        ..EngineConfig::new(nsh_testing::windowed_root::<Harness>())
    })
    .unwrap();
    app.wait_for("Tabs 1: 1[1*] 2[2*]").unwrap();
    // The second pane ends below the window: the grid has that much to
    // scroll.
    let (top, _) = rise(&app, "pane-1");
    let (below, height) = rise(&app, "pane-2");
    let hidden = below + height + GAP - 900.0;
    assert!(hidden > 50.0, "{hidden}");
    // The first pane's tab, held near the window's bottom edge, scrolls the
    // grid to its end; dropped there, it is on the second pane.
    let (tab_top, tab_height) = rise(&app, "pane-tab-1-1");
    let to_edge = 900.0 - 12.0 - (tab_top + tab_height / 2.0);
    app.drag_and_hold(
        "id:pane-tab-1-1",
        point(px(0.0), px(to_edge)),
        DragOptions::default(),
    )
    .unwrap();
    app.wait(std::time::Duration::from_millis(1000)).unwrap();
    let scrolled = top - rise(&app, "pane-1").0;
    assert!((scrolled - hidden).abs() < 0.01, "{scrolled} of {hidden}");
    app.drop_held().unwrap();
    app.wait_for("Tabs 1: 1[] 2[2 1*]").unwrap();
    // Held near the grid's top edge, a tab scrolls it back.
    let (tab_top, tab_height) = rise(&app, "pane-tab-2-1");
    let to_edge = rise(&app, "pane-1").0 + hidden - GAP + 12.0 - (tab_top + tab_height / 2.0);
    app.drag_and_hold(
        "id:pane-tab-2-1",
        point(px(0.0), px(to_edge)),
        DragOptions::default(),
    )
    .unwrap();
    app.wait(std::time::Duration::from_millis(1000)).unwrap();
    assert!((rise(&app, "pane-1").0 - top).abs() < 0.01);
}

/// A pane as narrow as a resize makes it, holding more tabs than fit, and
/// a button that adds one more.
#[component]
fn NarrowHarness() -> impl IntoElement {
    let mut narrow = saved_workspace();
    for _ in 0..3 {
        add_tab(&mut narrow, 1, WidgetKind::MarketScreener);
    }
    let rect = active_grid(&narrow).layouts[4][0].rect;
    active_grid_mut(&mut narrow).layouts[4][0].rect =
        grid::resized(rect, -10000.0, 0.0, WIDTH, columns(4));
    let saved = signal(narrow);
    view()
        .size_full()
        .flex_col()
        .child(view().h(px(24.0)).flex_none().child(
            ui::button("Add a tab", ui::Tone::Normal).on_press(move || {
                saved.update(|state| add_tab(state, 1, WidgetKind::MarketScreener));
            }),
        ))
        .child(workspace(saved, crate::widgets::picker::Picker::new()))
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

#[test]
fn a_narrow_pane_keeps_its_controls_and_scrolls_its_tabs() {
    let mut app = Headless::start(EngineConfig {
        viewport: Size::new(px(1280.0), px(900.0)),
        ..EngineConfig::new(nsh_testing::windowed_root::<NarrowHarness>())
    })
    .unwrap();
    app.wait_for("id:pane-tab-1-5").unwrap();
    let (pane_left, pane_width) = span(&app, "pane-1");
    assert!(pane_width >= grid::MIN_PANE_WIDTH, "{pane_width}");
    let pane_right = pane_left + pane_width;

    // The controls are whole, at the pane's right edge.
    let mut left = pane_right - 1.0;
    for control in ["pane-move-1", "pane-settings-1"] {
        let (x, width) = span(&app, control);
        assert_eq!(width, 32.0, "{control}");
        assert!(x + width <= left && x > pane_left, "{control} at {x}");
        left = x;
    }
    // The tabs have the room left of them, less than the tabs take, with
    // the first tab whole at its start.
    let (strip_left, strip_width) = span(&app, "pane-tabs-1");
    assert_eq!(strip_left, pane_left + 1.0);
    assert_eq!(strip_left + strip_width, left);
    let (first_left, first_width) = span(&app, "pane-tab-1-1");
    assert_eq!(first_left, strip_left);
    assert!(first_width < strip_width, "{first_width} in {strip_width}");
    let tabs: f32 = [1, 3, 4, 5]
        .iter()
        .map(|tab| span(&app, &format!("pane-tab-1-{tab}")).1)
        .sum();
    assert!(tabs + 32.0 > strip_width + 50.0, "{tabs} in {strip_width}");

    // A wheel over the tabs moves them sideways, whichever way it turns.
    let first = |app: &Headless| span(app, "pane-tab-1-1").0;
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    wheel(
        &mut app,
        "pane-tab-row-1",
        WheelDelta::Lines(point(0.0, 3.0)),
    );
    assert!(near(first(&app), strip_left - 90.0), "{}", first(&app));
    wheel(
        &mut app,
        "pane-tab-row-1",
        WheelDelta::Lines(point(0.0, -1.0)),
    );
    assert!(near(first(&app), strip_left - 60.0), "{}", first(&app));
    let sideways = WheelDelta::Points(point(px(-200.0), px(5.0)));
    wheel(&mut app, "pane-tab-row-1", sideways);
    assert!(near(first(&app), strip_left), "{}", first(&app));
    // A drag pans them too, and the wheel goes on from where it left them.
    app.scroll("id:pane-tabs-1", px(40.0)).unwrap();
    wheel(
        &mut app,
        "pane-tab-row-1",
        WheelDelta::Lines(point(0.0, 1.0)),
    );
    assert!(near(first(&app), strip_left - 70.0), "{}", first(&app));

    // A tab added shows at once: the tabs are then at their end, and the
    // wheel goes back from there.
    app.press("Add a tab").unwrap();
    app.wait_for("id:pane-tab-1-6").unwrap();
    app.wait(std::time::Duration::from_millis(200)).unwrap();
    assert!(app.scroll("id:pane-tabs-1", px(40.0)).is_err());
    let end = first(&app);
    wheel(
        &mut app,
        "pane-tab-row-1",
        WheelDelta::Lines(point(0.0, -1.0)),
    );
    assert!(near(first(&app), end + 30.0), "{} from {end}", first(&app));

    // Tabs that all show leave the wheel to the page: they listen for it
    // only while they reach past their room.
    let listens = std::sync::Arc::new(std::sync::Mutex::new(HashMap::new()));
    app.observe({
        let listens = listens.clone();
        move |_, batch: &nsh_core::MountBatch| {
            for item in &batch.items {
                if let nsh_core::MountItem::Events { id, events, .. } = item {
                    let wheel = events.contains(EventKind::Wheel);
                    listens.lock().unwrap().insert(*id, wheel);
                }
            }
        }
    });
    let hears_wheel = |app: &Headless| {
        let row = node(app, "pane-tab-row-1");
        listens.lock().unwrap().get(&row).copied()
    };
    app.drag(
        "id:pane-resize-1",
        point(px(900.0), px(0.0)),
        DragOptions::default(),
    )
    .unwrap();
    app.wait(std::time::Duration::from_millis(200)).unwrap();
    assert!(near(first(&app), span(&app, "pane-tabs-1").0));
    assert_ne!(hears_wheel(&app), Some(true));
    app.drag(
        "id:pane-resize-1",
        point(px(-900.0), px(0.0)),
        DragOptions::default(),
    )
    .unwrap();
    app.wait(std::time::Duration::from_millis(200)).unwrap();
    assert_eq!(hears_wheel(&app), Some(true));
}

/// The narrow pane's app, once its tabs show.
fn narrow_app() -> Headless {
    let mut app = Headless::start(EngineConfig {
        viewport: Size::new(px(1280.0), px(900.0)),
        ..EngineConfig::new(nsh_testing::windowed_root::<NarrowHarness>())
    })
    .unwrap();
    app.wait_for("id:pane-tab-1-5").unwrap();
    app
}

/// How far the middle of the other pane's tab is from a point `inset` into
/// the narrow pane's tabs (from their end when negative).
fn to_tabs_edge(app: &Headless, inset: f32) -> Point<Points> {
    let (strip_left, strip_width) = span(app, "pane-tabs-1");
    let (tab_left, tab_width) = span(app, "pane-tab-2-2");
    let edge = if inset < 0.0 {
        strip_left + strip_width
    } else {
        strip_left
    };
    point(
        px(edge + inset - (tab_left + tab_width / 2.0)),
        px(-30.0 * ROW_STEP),
    )
}

#[test]
fn tabs_that_reach_past_their_room_pan_under_a_touch_and_scroll_under_a_tab_in_hand() {
    let mut app = narrow_app();
    let first = |app: &Headless| span(app, "pane-tab-1-1").0;
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    let settle = |app: &mut Headless, ms| app.wait(std::time::Duration::from_millis(ms)).unwrap();
    let (strip_left, strip_width) = span(&app, "pane-tabs-1");
    assert_eq!(first(&app), strip_left);

    // A touch dragged sideways over them pans them, and picks no tab up...
    app.drag_and_hold(
        "id:pane-tab-1-3",
        point(px(-60.0), px(0.0)),
        DragOptions::default(),
    )
    .unwrap();
    settle(&mut app, 200);
    assert!(first(&app) < strip_left - 20.0, "{}", first(&app));
    assert!(!app.screen().matches("id:tab-drag-ghost").unwrap());
    app.drop_held().unwrap();
    // ...and one pulled out of them does: the tab goes to the pane below.
    app.drag(
        "id:pane-tab-1-3",
        point(px(0.0), px(30.0 * ROW_STEP)),
        DragOptions::default(),
    )
    .unwrap();
    app.wait_for("id:pane-tab-2-3").unwrap();
    assert_eq!(app.screen().count("id:pane-tab-1-3").unwrap(), 0);

    // A tab held over the last points of the tabs scrolls them on under it,
    // to their end, where the pointer is past the last tab: it goes after it.
    let tabs: f32 = [1, 4, 5]
        .iter()
        .map(|tab| span(&app, &format!("pane-tab-1-{tab}")).1)
        .sum();
    let hidden = tabs + 32.0 - strip_width;
    assert!(hidden > 50.0, "{hidden}");
    let by = to_tabs_edge(&app, -10.0);
    app.drag_and_hold("id:pane-tab-2-2", by, DragOptions::default())
        .unwrap();
    settle(&mut app, 1000);
    assert!(near(first(&app), strip_left - hidden), "{}", first(&app));
    app.wait_for("id:pane-drop-1-3").unwrap();
    app.drop_held().unwrap();
    app.wait_for("id:pane-tab-1-2").unwrap();
    assert_eq!(app.screen().count("id:pane-tab-2-2").unwrap(), 0);

    // Held over their first points, tabs at their end scroll back to their
    // start, where the tab goes before the first.
    let mut app = narrow_app();
    wheel(
        &mut app,
        "pane-tab-row-1",
        WheelDelta::Lines(point(0.0, 100.0)),
    );
    assert!(first(&app) < strip_left - 50.0, "{}", first(&app));
    let by = to_tabs_edge(&app, 10.0);
    app.drag_and_hold("id:pane-tab-2-2", by, DragOptions::default())
        .unwrap();
    settle(&mut app, 1000);
    assert!(near(first(&app), strip_left), "{}", first(&app));
    app.wait_for("id:pane-drop-1-0").unwrap();
    app.drop_held().unwrap();
    app.wait_for("id:pane-tab-1-2").unwrap();
    settle(&mut app, 200);
    let order: Vec<f32> = [2, 1, 3, 4, 5]
        .iter()
        .map(|tab| span(&app, &format!("pane-tab-1-{tab}")).0)
        .collect();
    assert!(order.is_sorted(), "{order:?}");
}

/// The view with a test id.
fn node(app: &Headless, test_id: &str) -> nsh_core::NodeId {
    let nodes = app.screen().layout_nodes().into_iter();
    nodes
        .filter(|node| node.test_id.as_deref() == Some(test_id))
        .map(|node| node.id)
        .next()
        .unwrap_or_else(|| panic!("no view `{test_id}`"))
}

/// A wheel turned by `delta` over the view with a test id.
fn wheel(app: &mut Headless, test_id: &str, delta: WheelDelta) {
    let at = point(px(0.0), px(0.0));
    let event = WheelEvent::new(at, at, delta, Default::default());
    app.engine().control().dispatch_event(
        node(app, test_id),
        EventKind::Wheel,
        EventPayload::Wheel(event),
    );
    app.wait(std::time::Duration::from_millis(100)).unwrap();
}
