use super::*;
use nsh_core::element::Root;
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

const WIDTH: f32 = 1264.0;

fn saved_workspace() -> StoredWorkspace {
    let mut saved = StoredWorkspace::default();
    normalize(&mut saved);
    super::spawn(&mut saved);
    super::spawn(&mut saved);
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
    super::spawn(&mut saved);
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
        |s| add_tab(s, 1),
        |s| remove_pane(s, 1),
        super::spawn,
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
        .child(workspace(saved))
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
