use super::*;
use nsh_core::element::Root;
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

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
        .child(workspace(saved, crate::widgets::picker::Picker::new()))
}

#[test]
fn responder_survives_live_updates_and_commits_on_release() {
    let mut app = Headless::start(EngineConfig {
        viewport: Size::new(px(1280.0), px(900.0)),
        ..EngineConfig::new(Root::of::<Harness>())
    })
    .unwrap();
    app.wait_for("Saved 0 0 24 30").unwrap();
    app.drag_and_hold("id:pane-resize-1", point(px(0.0), px(64.0)))
        .unwrap();
    assert!(
        app.shows("Saved 0 0 24 30"),
        "preview must not change committed state"
    );
    app.drop_held().unwrap();
    app.wait_for("Saved 0 0 24 34").unwrap();
    // A second gesture also proves the first responder was released.
    app.drag("id:pane-move-1", point(px(106.0), px(32.0)))
        .unwrap();
    app.wait_for("Saved 4 2 24 34").unwrap();
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
        ..EngineConfig::new(Root::of::<NarrowHarness>())
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
    app.drag("id:pane-resize-1", point(px(900.0), px(0.0)))
        .unwrap();
    app.wait(std::time::Duration::from_millis(200)).unwrap();
    assert!(near(first(&app), span(&app, "pane-tabs-1").0));
    assert_ne!(hears_wheel(&app), Some(true));
    app.drag("id:pane-resize-1", point(px(-900.0), px(0.0)))
        .unwrap();
    app.wait(std::time::Duration::from_millis(200)).unwrap();
    assert_eq!(hears_wheel(&app), Some(true));
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
