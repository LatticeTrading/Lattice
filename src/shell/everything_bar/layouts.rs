//! The Layout Manager's default layouts (the old app's Community Layouts
//! panel): a gallery of the templates ([`templates::ALL`]), each a preview
//! of where its widgets go, its name and what it is for. Pressing one
//! starts a page from it ([`templates::apply`]) and closes the panel: the
//! active page when it has no widget, a new page otherwise, so no page
//! loses what it had.
//!
//! It takes the Layout Manager's place below the bar; "Back" returns to
//! the manager.

use nsh_core::{LineHeight, prelude::*};

use super::{BAR_HEIGHT, Manager, persist};
use crate::{
    layout::templates::{self, Template},
    storage::StoredWorkspace,
    theme,
    ui::{self, Side, Tone},
};

/// The old panel's width: two previews side by side.
const PANEL_WIDTH: Points = px(620.0);
/// Windows narrower than this get one preview a row, in a panel as wide as
/// the Layout Manager.
pub(super) const NARROW_BELOW: f32 = 640.0;
const PREVIEW_HEIGHT: Points = px(112.0);
/// What is above and below the gallery's scrolling part when the panel is
/// where it opens: the shell's padding, the bar, the panel's title and
/// "Back", and as much again as the padding under the panel.
const AROUND_GALLERY: f32 = 8.0 + BAR_HEIGHT.0 + 40.0 + ui::ROW_HEIGHT.0 + 12.0;

/// The gallery, below `anchor`; `narrow` for a window with room for one
/// preview a row.
pub(super) fn panel(
    workspace: Signal<StoredWorkspace>,
    manager: Manager,
    narrow: bool,
    anchor: &ElementRef,
) -> Overlay {
    let back = ui::button("‹ Back to Layout Manager", Tone::Normal)
        .h(ui::ROW_HEIGHT)
        .w_full()
        .flex_none()
        .border_b_1()
        .border_color(theme::BORDER)
        .label("Back to Layout Manager")
        .on_press(move || manager.browse(false));

    let intro = view().p(px(10.0)).text_color(theme::TEXT_MUTED).child(
        text(
            "Choose a layout to open on a new page, or on the current page if it has no \
                 widgets. Your other pages stay unchanged.",
        )
        .line_height(LineHeight::Multiple(1.5)),
    );

    // Only a layout that could not be saved has anything to say: one that
    // was closes the panel.
    let error = dynamic(move || {
        let feedback = manager.feedback.get().filter(|feedback| feedback.error)?;
        Some(
            view()
                .px(px(10.0))
                .pb(px(10.0))
                .text_color(theme::DANGER)
                .role(AccessibilityRole::Alert)
                .child(text(feedback.message)),
        )
    });

    let apply = move |template: &'static Template| {
        let mut state = workspace.get_untracked();
        templates::apply(&mut state, template, workspace_height());
        match persist(&state) {
            Ok(()) => batch(|| {
                workspace.set(state);
                manager.close();
            }),
            Err(error) => manager.report(true, error),
        }
    };

    let mut gallery = view()
        .grid()
        .grid_cols(if narrow { 1 } else { 2 })
        .gap(px(12.0))
        .px(px(10.0))
        .pb(px(12.0))
        .role(AccessibilityRole::List)
        .label("Default layouts");
    for template in templates::ALL {
        gallery = gallery.child(card(template, apply));
    }

    let body = view()
        .test_id("default-layouts")
        .flex_col()
        .text_size(px(11.0))
        .child(back)
        .child(
            scroll()
                .max_h(move || px((window().height.0 - AROUND_GALLERY).max(160.0)))
                .child(view().flex_col().child(intro).child(error).child(gallery)),
        );

    ui::floating_panel(
        anchor,
        Side::Left,
        if narrow {
            super::PANEL_WIDTH
        } else {
            PANEL_WIDTH
        },
        "Default Layouts",
        move || manager.close(),
        body,
    )
}

/// The workspace's height: the window's, less the shell's padding around
/// it and the bar above it.
fn workspace_height() -> f32 {
    window().height.0 - 16.0 - BAR_HEIGHT.0
}

/// One template: its preview, its name and what it is for. The whole card
/// is the button, and lights its preview's edge under the pointer.
fn card(template: &'static Template, on_apply: impl Fn(&'static Template) + 'static) -> View {
    let hovered = signal(false);
    view()
        .min_w(px(0.0))
        .flex_col()
        .role(AccessibilityRole::Button)
        .label(format!("Apply {} layout", template.name))
        .hint(template.description)
        .test_id(&format!("default-layout-{}", template.id))
        .on_mouse_enter(move |_: &MouseEvent| hovered.set(true))
        .on_mouse_leave(move |_: &MouseEvent| hovered.set(false))
        .on_press(move || on_apply(template))
        .child(preview(template, hovered))
        .child(
            view()
                .pt(px(8.0))
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .child(text(template.name)),
        )
        .child(
            view()
                .pt(px(4.0))
                .text_color(theme::TEXT_MUTED)
                .child(text(template.description).line_height(LineHeight::Multiple(1.5))),
        )
}

/// The template's design, a box per widget where the widget goes.
fn preview(template: &Template, hovered: Signal<bool>) -> View {
    let line = |edge: u32| i16::try_from(edge + 1).unwrap_or(i16::MAX);
    let mut preview = view()
        .test_id(&format!("default-layout-{}-preview", template.id))
        .w_full()
        .h(PREVIEW_HEIGHT)
        .grid()
        .grid_cols(templates::COLUMNS as u16)
        .grid_rows(templates::ROWS as u16)
        .gap(px(4.0))
        .p(px(6.0))
        .border_1()
        .border_color(theme::BORDER)
        .style_when(
            move || hovered.get(),
            |style| style.border_color(theme::ACCENT),
        )
        .bg(theme::BACKGROUND);
    for panel in template.panels {
        preview = preview.child(
            view()
                .col_start(line(panel.x))
                .col_end(line(panel.x + panel.w))
                .row_start(line(panel.y))
                .row_end(line(panel.y + panel.h))
                .min_w(px(0.0))
                .min_h(px(0.0))
                .flex()
                .items_center()
                .justify_center()
                .overflow_hidden()
                .border_1()
                .border_color(theme::BORDER)
                .bg(theme::SURFACE)
                .text_size(px(10.0))
                .text_color(theme::TEXT_MUTED)
                .child(text(panel.label).number_of_lines(1)),
        );
    }
    preview
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nsh_engine::EngineConfig;
    use nsh_testing::{Headless, MemoryStorage, WAIT};

    use super::*;
    use crate::{
        layout,
        shell::{clock::Clock, everything_bar::bar},
        theme::ActiveTheme,
        widgets::picker::Picker,
    };

    const MANAGER: &str = "id:layout-manager";
    const GALLERY: &str = "id:default-layouts";

    /// The bar over the saved workspace (one blank page when nothing is
    /// saved), with a line reporting the committed workspace.
    #[component]
    fn Harness() -> impl IntoElement {
        let mut saved = crate::storage::load();
        layout::normalize(&mut saved);
        let workspace = signal(saved);
        view()
            .size_full()
            .flex_col()
            .child(bar(
                workspace,
                ActiveTheme::restore(),
                Clock::restore(),
                Picker::new(),
            ))
            .child(text(move || {
                let state = workspace.get();
                format!(
                    "Pages {} active {} widgets {}",
                    state.pages.len(),
                    state.active_page_id,
                    layout::widget_ids(&state).len()
                )
            }))
    }

    fn start(width: f32) -> Headless {
        launch(MemoryStorage::new([]), width, "Pages 1 active 1 widgets 0")
    }

    /// The bar over what `storage` holds, once it reports `workspace`.
    fn launch(storage: Arc<MemoryStorage>, width: f32, workspace: &str) -> Headless {
        let mut app = Headless::start(EngineConfig {
            viewport: Size::new(px(width), px(800.0)),
            storage: Some(storage),
            ..EngineConfig::new(nsh_testing::windowed_root::<Harness>())
        })
        .unwrap();
        app.wait_for(workspace).unwrap();
        app
    }

    fn browse(app: &mut Headless) {
        app.press("Layout manager").unwrap();
        app.wait_for(MANAGER).unwrap();
        app.press("Browse default layouts").unwrap();
        app.wait_for(GALLERY).unwrap();
        assert!(!app.screen().matches(MANAGER).unwrap());
    }

    #[test]
    fn the_gallery_offers_every_default_layout_at_any_width() {
        for width in [1280.0, 480.0] {
            let mut app = start(width);
            browse(&mut app);
            for template in templates::ALL {
                let card = format!("label:Apply {} layout", template.name);
                assert!(app.screen().matches(&card).unwrap(), "{card} at {width}");
                assert!(app.shows(template.description), "{card} at {width}");
            }
            // Two to a row where the window has room, one where it has
            // not; either way the panel is inside the window.
            let card = |at: usize| {
                let id = format!("default-layout-{}", templates::ALL[at].id);
                placed(&app, &id)
            };
            let (first, second) = (card(0), card(1));
            if width < NARROW_BELOW {
                assert_eq!(second.origin.x, first.origin.x);
                assert!(second.origin.y >= first.bottom());
            } else {
                assert_eq!(second.origin.y, first.origin.y);
                assert!(second.origin.x >= first.right());
            }
            let gallery = placed(&app, "default-layouts");
            assert!(gallery.origin.x.0 >= 0.0 && gallery.right().0 <= width);
        }
    }

    #[test]
    fn a_default_layout_opens_on_a_blank_page_and_on_a_new_one_after_it() {
        let storage = MemoryStorage::new([]);
        let mut app = launch(storage.clone(), 1280.0, "Pages 1 active 1 widgets 0");
        let (first, second) = (&templates::ALL[0], &templates::ALL[2]);

        browse(&mut app);
        app.press(&format!("Apply {} layout", first.name)).unwrap();
        app.wait_for(&format!("Pages 1 active 1 widgets {}", first.panels.len()))
            .unwrap();
        // Applying closes the panel, and the next opening is the manager's.
        app.wait_for_gone(GALLERY, WAIT).unwrap();

        browse(&mut app);
        app.press(&format!("Apply {} layout", second.name)).unwrap();
        let both = format!(
            "Pages 2 active 2 widgets {}",
            first.panels.len() + second.panels.len()
        );
        app.wait_for(&both).unwrap();

        // Both pages are saved: the next launch opens with them.
        drop(app);
        launch(storage, 1280.0, &both);
    }

    #[test]
    fn back_returns_to_the_manager_and_closing_the_gallery_closes_both() {
        let mut app = start(1280.0);
        browse(&mut app);
        app.press("Back to Layout Manager").unwrap();
        app.wait_for(MANAGER).unwrap();
        assert!(!app.screen().matches(GALLERY).unwrap());

        browse_again(&mut app);
        app.press("Close Default Layouts").unwrap();
        app.wait_for_gone(GALLERY, WAIT).unwrap();
        assert!(!app.screen().matches(MANAGER).unwrap());

        // The bar's button opens the manager, not the gallery it left.
        app.press("Layout manager").unwrap();
        app.wait_for(MANAGER).unwrap();
        assert!(!app.screen().matches(GALLERY).unwrap());

        browse_again(&mut app);
        app.press_key("Escape").unwrap();
        app.wait_for_gone(GALLERY, WAIT).unwrap();
        assert!(app.shows("Pages 1 active 1 widgets 0"));
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
    fn a_preview_draws_each_widget_where_its_template_puts_it() {
        let mut app = start(1280.0);
        browse(&mut app);
        let nodes = app.screen().layout_nodes();
        // A box's edge, to the point layout rounds it to.
        let near = |at: Points, expected: f32| (at.0 - expected).abs() <= 1.0;
        for template in templates::ALL {
            let id = format!("default-layout-{}-preview", template.id);
            let preview = nodes
                .iter()
                .find(|node| node.test_id.as_deref() == Some(id.as_str()))
                .unwrap_or_else(|| panic!("{id} is not on screen"));
            // Inside its border and padding, a gap between the cells.
            let (left, top) = (
                preview.bounds.origin.x.0 + 7.0,
                preview.bounds.origin.y.0 + 7.0,
            );
            let column = (preview.bounds.size.width.0 - 14.0 + 4.0) / templates::COLUMNS as f32;
            let row = (preview.bounds.size.height.0 - 14.0 + 4.0) / templates::ROWS as f32;
            let boxes: Vec<_> = nodes
                .iter()
                .filter(|node| node.parent == preview.id)
                .map(|node| node.bounds)
                .collect();
            assert_eq!(boxes.len(), template.panels.len(), "{id}");
            for (panel, placed) in template.panels.iter().zip(boxes) {
                assert!(
                    near(placed.origin.x, left + panel.x as f32 * column)
                        && near(placed.origin.y, top + panel.y as f32 * row)
                        && near(placed.size.width, panel.w as f32 * column - 4.0)
                        && near(placed.size.height, panel.h as f32 * row - 4.0),
                    "{} of {id} at {placed:?}",
                    panel.label
                );
            }
        }
    }

    /// From the open manager to the gallery.
    fn browse_again(app: &mut Headless) {
        app.press("Browse default layouts").unwrap();
        app.wait_for(GALLERY).unwrap();
    }
}
