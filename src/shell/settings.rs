//! The Settings panel, which the Everything Bar's gear opens: the user's
//! preferences, which for now is the theme.

use nsh_base::{SelectItem, select};
use nsh_core::prelude::*;

use crate::{
    theme::{self, ActiveTheme},
    ui::{self, Side, Tone},
};

const PANEL_WIDTH: Points = px(300.0);

/// The Settings panel, below the right end of `anchor`.
pub(super) fn panel(
    active: ActiveTheme,
    anchor: &ElementRef,
    on_close: impl Fn() + Clone + 'static,
) -> Overlay {
    let body = view()
        .test_id("settings")
        .flex_col()
        .child(theme_row(active));
    ui::floating_panel(anchor, Side::Right, PANEL_WIDTH, "Settings", on_close, body)
}

/// "Theme", then the previous theme button, a dropdown of every theme
/// showing the current one, and the next theme button. The buttons wrap
/// around at either end of the list.
fn theme_row(active: ActiveTheme) -> View {
    let trigger = view()
        .size_full()
        .flex()
        .items_center()
        .gap(px(6.0))
        .px(px(10.0))
        .text_color(theme::TEXT)
        .hover(|style| style.bg(theme::SURFACE))
        .active(|style| style.bg(theme::BORDER))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .child(text(move || active.get().name.clone())),
        )
        .child(text("▾").text_color(theme::TEXT_FAINT));

    let items: Vec<SelectItem> = theme::all()
        .iter()
        .map(|theme| SelectItem::new(theme.id, theme.name.clone()))
        .collect();

    let picker = select()
        .items(items)
        .value(move || SharedString::from(active.get().id))
        .on_change(move |id: &Option<SharedString>| {
            if let Some(id) = id {
                active.select(id);
            }
        })
        .accessibility_label("Theme")
        .trigger(trigger)
        .popup_style(|style| {
            style
                .py(px(4.0))
                .bg(theme::PANEL)
                .border_1()
                .border_color(theme::BORDER)
                .text_color(theme::TEXT)
                .text_size(px(11.0))
        })
        .item_style(|style| style.h(px(30.0)).px(px(10.0)).flex().items_center())
        // The rows' states are classes; every role is a Tailwind color.
        .item_class("focus:bg-surface selected:text-accent")
        .flex_1()
        .min_w(px(0.0))
        .h_full()
        .border_l_1()
        .border_color(theme::BORDER);

    view()
        .h(ui::ROW_HEIGHT)
        .w_full()
        .flex_row()
        .items_center()
        .text_size(px(11.0))
        .child(
            view()
                .px(px(10.0))
                .text_color(theme::TEXT_MUTED)
                .child(text("Theme")),
        )
        .child(step(active, -1, "‹", "Previous theme"))
        .child(picker)
        .child(step(active, 1, "›", "Next theme"))
}

/// A square button that switches `by` themes along the list.
fn step(active: ActiveTheme, by: isize, glyph: &str, label: &'static str) -> View {
    ui::button(glyph, Tone::Normal)
        .w(ui::ROW_HEIGHT)
        .flex_none()
        .justify_center()
        .border_l_1()
        .border_color(theme::BORDER)
        .label(label)
        .on_press(move || active.step(by))
}
