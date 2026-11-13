use std::sync::Arc;

use super::*;
use nsh_core::element::Root;
use nsh_engine::EngineConfig;
use nsh_testing::{Headless, MemoryStorage, WAIT};

const CODE: &str = "id:layout-code";

/// The bar over a two-page workspace, with a line reporting the committed
/// workspace so tests can observe what the Layout Manager changed.
#[component]
fn Harness() -> impl IntoElement {
    let mut saved = StoredWorkspace::default();
    layout::normalize(&mut saved);
    layout::add_page(&mut saved);
    let workspace = signal(saved);
    view()
        .size_full()
        .flex_col()
        .child(bar(
            workspace,
            ActiveTheme::restore(),
            Clock::restore(),
            crate::widgets::picker::Picker::new(),
        ))
        .child(text(move || {
            let state = workspace.get();
            format!(
                "Pages {} active {}",
                state.pages.len(),
                state.active_page_id
            )
        }))
}

fn start() -> Headless {
    start_with(MemoryStorage::new([]))
}

fn start_with(storage: Arc<MemoryStorage>) -> Headless {
    let mut app = Headless::start(EngineConfig {
        viewport: Size::new(px(1280.0), px(800.0)),
        storage: Some(storage),
        ..EngineConfig::new(Root::of::<Harness>())
    })
    .unwrap();
    app.wait_for("Pages 2 active 2").unwrap();
    app
}

fn open(app: &mut Headless) {
    app.press("Layout manager").unwrap();
    app.wait_for("id:layout-manager").unwrap();
}

#[test]
fn close_button_layout_button_and_escape_close_the_manager() {
    let mut app = start();
    open(&mut app);
    app.press("Close Layout Manager").unwrap();
    app.wait_for_gone("id:layout-manager", WAIT).unwrap();

    open(&mut app);
    app.press("Layout manager").unwrap();
    app.wait_for_gone("id:layout-manager", WAIT).unwrap();

    open(&mut app);
    app.press_key("Escape").unwrap();
    app.wait_for_gone("id:layout-manager", WAIT).unwrap();
}

#[test]
fn clear_asks_first_then_deletes_and_finally_empties_the_last_page() {
    let mut app = start();
    open(&mut app);
    let question = "Delete layout 2 and its widgets? This cannot be undone.";

    app.press("Clear layout 2").unwrap();
    app.wait_for(question).unwrap();
    app.press("Cancel").unwrap();
    app.wait_for_gone(question, WAIT).unwrap();
    assert!(app.shows("Pages 2 active 2"));

    app.press("Clear layout 2").unwrap();
    app.press("Delete layout").unwrap();
    app.wait_for("Pages 1 active 1").unwrap();
    app.wait_for("Layout 2 cleared.").unwrap();

    app.press("Clear layout 1").unwrap();
    app.wait_for("Remove every widget from layout 1? This cannot be undone.")
        .unwrap();
    app.press("Delete layout").unwrap();
    app.wait_for("Layout 1 cleared.").unwrap();
    assert!(app.shows("Pages 1 active 1"));
}

#[test]
fn generated_codes_import_after_confirmation() {
    let mut app = start();
    open(&mut app);
    app.press("Generate layout code").unwrap();
    app.wait_for("Layout code generated and copied.").unwrap();
    let code = app.screen().input_text(CODE).unwrap().unwrap().to_owned();
    assert!(code.starts_with("LATTICE1."));
    assert_eq!(app.screen().clipboard(), Some(code.as_str()));

    app.press("Add layout page").unwrap();
    app.wait_for("Pages 3 active 3").unwrap();
    app.press("Import layout").unwrap();
    app.wait_for("Replace layouts").unwrap();
    assert!(
        app.shows("Pages 3 active 3"),
        "import must wait for confirmation"
    );
    app.press("Replace layouts").unwrap();
    app.wait_for("Pages 2 active 2").unwrap();
    app.wait_for("Layout imported.").unwrap();
}

#[test]
fn the_code_field_is_editable_and_rejects_invalid_codes() {
    let mut app = start();
    open(&mut app);
    assert!(!app.screen().enabled("Import layout").unwrap());

    // Return in the field imports, like the button.
    app.type_text(CODE, "nope\n").unwrap();
    assert_eq!(app.screen().input_text(CODE).unwrap(), Some("nope"));
    app.wait_for("Code must start with LATTICE1.").unwrap();
    assert!(app.screen().enabled("Import layout").unwrap());

    // Editing again clears the error.
    app.type_text(CODE, "!").unwrap();
    app.wait_for_gone("Code must start with LATTICE1.", WAIT)
        .unwrap();

    app.clear(CODE).unwrap();
    app.next_change(WAIT).ok();
    assert!(!app.screen().enabled("Import layout").unwrap());

    // The field keeps its text while the manager is closed.
    app.type_text(CODE, "LATTICE1.00").unwrap();
    app.press("Close Layout Manager").unwrap();
    app.wait_for_gone("id:layout-manager", WAIT).unwrap();
    open(&mut app);
    assert_eq!(app.screen().input_text(CODE).unwrap(), Some("LATTICE1.00"));
}

/// The color the bar's "Layout" button is drawn in: the theme's muted text.
fn muted(app: &Headless) -> nsh_core::Color {
    app.screen().span_style("text:Layout").unwrap().color
}

#[test]
fn settings_switch_the_theme_in_place_and_remember_it() {
    let storage = MemoryStorage::new([]);
    let mut app = start_with(storage.clone());
    app.press("Application settings").unwrap();
    app.wait_for("id:settings").unwrap();
    assert!(app.shows("Dark"));
    let dark = muted(&app);

    // The arrows step along the list and wrap at either end; what is open
    // repaints in place, the panel included.
    let last = theme::all().last().unwrap().name.as_str();
    app.press("Next theme").unwrap();
    app.wait_for("Light").unwrap();
    assert_ne!(muted(&app), dark);
    assert!(app.screen().matches("id:settings").unwrap());
    app.press("Previous theme").unwrap();
    app.wait_for("Dark").unwrap();
    assert_eq!(muted(&app), dark);
    app.press("Previous theme").unwrap();
    app.wait_for(last).unwrap();
    app.press("Next theme").unwrap();
    app.wait_for("Dark").unwrap();

    // The dropdown chooses directly. Escape closes its list before the panel.
    app.press("role:combobox[Theme]").unwrap();
    app.press("role:option[Nord]").unwrap();
    app.wait_for_gone("role:option[Nord]", WAIT).unwrap();
    app.wait_for("Nord").unwrap();
    let nord = muted(&app);
    assert_ne!(nord, dark);
    app.press("role:combobox[Theme]").unwrap();
    app.wait_for("role:option[Nord]").unwrap();
    app.press_key("Escape").unwrap();
    app.wait_for_gone("role:option[Nord]", WAIT).unwrap();
    assert!(app.screen().matches("id:settings").unwrap());
    app.press_key("Escape").unwrap();
    app.wait_for_gone("id:settings", WAIT).unwrap();

    // The choice is saved, and the next launch opens in it.
    drop(app);
    let app = start_with(storage);
    assert_eq!(muted(&app), nord);
}
