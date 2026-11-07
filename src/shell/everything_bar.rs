//! The shared Everything Bar.
//!
//! NSH's hidden desktop title bar supplies the native minimize,
//! maximize/restore, and close buttons. The same bar is a drag region on
//! desktop; the web build gets neither because the browser owns that chrome.

use nsh_base::input;
use nsh_core::{
    desktop::TitleBarStyle,
    element::{
        ImageSrc,
        desktop::{set_title, set_title_bar, title_bar},
    },
    prelude::*,
};

use crate::{layout, storage, storage::StoredWorkspace, theme};

pub const BAR_HEIGHT: Points = px(40.0);

pub fn bar(workspace: Signal<StoredWorkspace>) -> impl IntoElement {
    set_title("Lattice");
    if os().is_desktop() {
        set_title_bar(TitleBarStyle::Hidden, Some(BAR_HEIGHT));
    }

    dynamic(move || {
        let saved = workspace.get();
        let compact = window().width.0 <= 768.0;
        let control_width = if compact { 36.0 } else { 40.0 };
        let page_width = if compact { 28.0 } else { 32.0 };

        let wordmark = view()
            .h_full()
            .flex()
            .items_center()
            .flex_shrink_0()
            .px(px(if compact { 12.0 } else { 16.0 }))
            .text_size(px(if compact { 12.0 } else { 14.0 }))
            .font_weight(FontWeight::BOLD)
            .window_drag(os().is_desktop())
            .child(text("LATTICE"));

        let layout_placeholder = segment("Layout", page_width).disabled(true).label("Layout");

        let pages = saved
            .pages
            .iter()
            .enumerate()
            .map(|(index, page)| {
                let id = page.id;
                let selected = id == saved.active_page_id;
                segment(&(index + 1).to_string(), page_width)
                    .accessibility_state(move || AccessibilityState {
                        selected,
                        ..Default::default()
                    })
                    .text_color(if selected {
                        theme::accent()
                    } else {
                        theme::secondary()
                    })
                    .bg(if selected {
                        theme::accent().opacity(0.10)
                    } else {
                        theme::surface()
                    })
                    .on_press(move || {
                        workspace.update(|state| {
                            layout::select_page(state, id);
                            let _ = persist(state);
                        });
                    })
                    .child(text((index + 1).to_string()))
            })
            .collect::<Vec<_>>();

        let add_page = segment("+", page_width).label("Add layout page").on_press({
            let workspace = workspace;
            move || {
                workspace.update(|state| {
                    layout::add_page(state);
                    let _ = persist(state);
                });
            }
        });

        let left = view()
            .h_full()
            .flex()
            .items_center()
            .flex_shrink_0()
            .child(wordmark)
            .child(layout_placeholder)
            .children(pages)
            .child(add_page);

        let search = input()
            .h(px(38.0))
            .flex_1()
            .min_w(px(80.0))
            .px(px(8.0))
            .placeholder("Search symbols...")
            .readonly(true)
            .disabled(true)
            .text_size(px(if compact { 11.0 } else { 12.0 }))
            .border_l_1()
            .border_color(theme::border())
            .label("Search symbols");

        let new_widget = view()
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(if compact { 12.0 } else { 16.0 }))
            .border_l_1()
            .border_r_1()
            .border_color(theme::border())
            .text_color(theme::secondary())
            .label("New Widget")
            .on_press({
                let workspace = workspace;
                move || {
                    workspace.update(|state| {
                        layout::spawn(state);
                        let _ = persist(state);
                    });
                }
            })
            .child(text("+"))
            .child(text("New Widget").text_size(px(12.0)));

        let clock = view()
            .h_full()
            .flex()
            .items_center()
            .flex_shrink_0()
            .px(px(if compact { 12.0 } else { 16.0 }))
            .text_size(px(if compact { 11.0 } else { 12.0 }))
            .font_weight(FontWeight::MEDIUM)
            .window_drag(os().is_desktop())
            .label("Clock unavailable")
            .child(text("--:--:--"));

        let connection = view()
            .size(px(14.0))
            .pointer_events(PointerEvents::None)
            .child(
                svg(ImageSrc::svg_markup(include_str!(
                    "../../assets/icons/connection-status.svg"
                )))
                .size_full(),
            );
        let connection = icon_segment(control_width)
            .disabled(true)
            .label("Connection status")
            .child(connection);
        let settings = icon_segment(control_width)
            .disabled(true)
            .label("Application settings")
            .child(text("⚙"));

        let center = view()
            .flex_1()
            .min_w(px(0.0))
            .max_w(px(800.0))
            .h_full()
            .flex()
            .items_center()
            .child(search)
            .child(new_widget);

        // Native caption buttons are painted by NSH into the hidden title bar
        // on desktop. `title_bar().right` reserves their space; on web it is
        // zero, so these controls are omitted exactly as the browser requires.
        view()
            .test_id("everything-bar")
            .role(AccessibilityRole::Toolbar)
            .label("Everything Bar")
            .h(BAR_HEIGHT)
            .w_full()
            .flex_shrink_0()
            .flex_row()
            .items_center()
            .border_1()
            .border_color(theme::border())
            .bg(theme::surface())
            .window_drag(os().is_desktop())
            .pl(move || {
                if os().is_desktop() {
                    title_bar().left + px(0.0)
                } else {
                    px(0.0)
                }
            })
            .pr(move || {
                if os().is_desktop() {
                    title_bar().right + px(0.0)
                } else {
                    px(0.0)
                }
            })
            .child(left)
            .child(center)
            .child(clock)
            .child(connection)
            .child(settings)
    })
}

fn persist(state: &StoredWorkspace) -> Result<(), String> {
    storage::save(state).map_err(|error| error.to_string())
}

fn segment(label: &str, width: f32) -> View {
    icon_segment(width).child(text(label.to_owned()))
}

fn icon_segment(width: f32) -> View {
    view()
        .h_full()
        .w(px(width))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .px(px(8.0))
        .border_l_1()
        .border_color(theme::border())
        .text_color(theme::secondary())
        .text_size(px(12.0))
}
