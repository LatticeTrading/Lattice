use super::*;
use nsh_core::element::Root;
use nsh_engine::EngineConfig;
use nsh_testing::{Headless, WAIT};

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
        .child(bar(workspace))
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
    let mut app = Headless::start(EngineConfig {
        viewport: Size::new(px(1280.0), px(800.0)),
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
