//! Settings rows and their controls: rows with a name column or a name at
//! the edge, dropdowns, a check row and a checkbox.

use nsh_base::{SelectItem, select};
use nsh_core::{Checked, prelude::*};

use super::ROW_HEIGHT;
use crate::theme;

/// A settings row with its name in a fixed 78pt column, muted, and the
/// controls the caller adds right after it, filling the rest (the app's
/// settings and the screener's panel). Compare [`label_edge_row`], whose
/// control sits at the far edge.
pub fn label_column_row(name: &'static str) -> View {
    view()
        .h(ROW_HEIGHT)
        .w_full()
        .flex_row()
        .items_center()
        .text_size(px(11.0))
        .child(
            view()
                .w(px(78.0))
                .flex_none()
                .px(px(10.0))
                .text_color(theme::TEXT_MUTED)
                .child(text(name)),
        )
}

/// A row of the old app's settings panel (`.settings-row`): its name at
/// the left edge, on one line in primary text, and the control the caller
/// adds pushed to the right edge, 10pt in from both sides. Compare
/// [`label_column_row`], whose controls follow a fixed name column.
pub fn label_edge_row(name: &'static str) -> View {
    view()
        .w_full()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(10.0))
        .px(px(10.0))
        .py(px(6.0))
        .text_size(px(11.0))
        .child(form_label(name))
}

/// A [`label_edge_row`] whose control is a list: its name level with the
/// first entry (the old `.settings-row-list`).
pub fn label_edge_list_row(name: &'static str) -> View {
    view()
        .w_full()
        .flex_row()
        .items_start()
        .justify_between()
        .gap(px(10.0))
        .px(px(10.0))
        .py(px(6.0))
        .text_size(px(11.0))
        .child(form_label(name).pt(px(6.0)))
}

fn form_label(name: &'static str) -> Text {
    text(name)
        .flex_none()
        .number_of_lines(1)
        .text_size(px(11.0))
        .text_color(theme::TEXT)
}

/// A dropdown filling the rest of a [`label_column_row`]: it shows `shown`, lists `items`
/// (as they are while it is open),
/// marks the one whose id is `value`, and calls `on_pick` with a picked id.
pub fn dropdown(
    items: impl Fn() -> Vec<SelectItem> + 'static,
    value: impl Fn() -> SharedString + 'static,
    shown: impl Fn() -> String + 'static,
    on_pick: impl Fn(&str) + 'static,
    label: &'static str,
) -> impl IntoElement {
    let trigger = view()
        .size_full()
        .flex()
        .items_center()
        .gap(px(6.0))
        .px(px(10.0))
        .text_color(theme::TEXT)
        .hover(|style| style.bg(theme::SURFACE))
        .active(|style| style.bg(theme::BORDER))
        .child(view().flex_1().min_w(px(0.0)).child(text(shown)))
        .child(text("▾").text_color(theme::TEXT_FAINT));
    select_from(items, value, on_pick, label, trigger)
        .flex_1()
        .min_w(px(0.0))
        .h_full()
        .border_l_1()
        .border_color(theme::BORDER)
}

/// The old settings panel's select (`.settings-select`), the control of a
/// [`label_edge_row`]: a small bordered box as wide as what it shows (90pt
/// at least), which works as [`dropdown`] does.
pub fn compact_dropdown(
    items: impl Fn() -> Vec<SelectItem> + 'static,
    value: impl Fn() -> SharedString + 'static,
    shown: impl Fn() -> String + 'static,
    on_pick: impl Fn(&str) + 'static,
    label: &'static str,
) -> impl IntoElement {
    let trigger = view()
        .h(px(24.0))
        .min_w(px(90.0))
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .pl(px(8.0))
        .pr(px(6.0))
        .bg(theme::SURFACE)
        .border_1()
        .border_color(theme::BORDER)
        .text_size(px(11.0))
        .text_color(theme::TEXT)
        .hover(|style| style.border_color(theme::TEXT_FAINT))
        .child(text(shown).number_of_lines(1))
        .child(text("▾").text_color(theme::TEXT_FAINT));
    select_from(items, value, on_pick, label, trigger).flex_none()
}

/// The select both dropdowns are: `trigger` opens the list of `items`,
/// the one whose id is `value` marked, and a pick calls `on_pick`.
fn select_from(
    items: impl Fn() -> Vec<SelectItem> + 'static,
    value: impl Fn() -> SharedString + 'static,
    on_pick: impl Fn(&str) + 'static,
    label: &'static str,
    trigger: View,
) -> impl Styled + IntoElement {
    select()
        .items(items)
        .value(value)
        .on_change(move |id: &Option<SharedString>| {
            if let Some(id) = id {
                on_pick(id);
            }
        })
        .accessibility_label(label)
        .trigger(trigger)
        .popup_style(|style| {
            style
                .py(px(4.0))
                .max_h(px(320.0))
                .bg(theme::PANEL)
                .border_1()
                .border_color(theme::BORDER)
                .text_color(theme::TEXT)
                .text_size(px(11.0))
        })
        .item_style(|style| style.h(px(30.0)).px(px(10.0)).flex().items_center())
        // The rows' states are classes; every role is a Tailwind color.
        .item_class("focus:bg-surface selected:text-accent")
}

/// The old settings panel's checkbox (`.checkbox-box`), the control of a
/// [`label_edge_row`]: a 14pt box, filled with the accent around a small
/// square while `checked`. Pressing it calls `on_toggle`; `label` names it
/// for assistive tech.
pub fn check_box(
    label: &'static str,
    checked: impl Fn() -> bool + Copy + 'static,
    on_toggle: impl Fn() + 'static,
) -> View {
    let edge = move |unchecked: theme::Role| {
        move || {
            if checked() {
                theme::ACCENT.get()
            } else {
                unchecked.get()
            }
        }
    };
    view()
        .size(px(14.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .border_1()
        .border_color(edge(theme::BORDER))
        .bg(edge(theme::SURFACE))
        .role(AccessibilityRole::Checkbox)
        .label(label)
        .accessibility_state(move || AccessibilityState {
            checked: Some(if checked() {
                Checked::True
            } else {
                Checked::False
            }),
            ..Default::default()
        })
        .hover(move |style| style.border_color(edge(theme::TEXT_FAINT)))
        .on_press(on_toggle)
        .child(
            // What reads on the accent, as the check row's mark.
            view()
                .size(px(8.0))
                .bg(theme::BACKGROUND)
                .style_when(move || !checked(), |s| s.hidden()),
        )
}

/// A row that checks and unchecks an option: its name, then a box with a
/// check mark while `checked`. Pressing it calls `on_toggle`; while
/// `disabled` it takes no presses (an option that must stay on).
pub fn check_row(
    name: &'static str,
    checked: impl Fn() -> bool + Copy + 'static,
    disabled: impl Fn() -> bool + Copy + 'static,
    on_toggle: impl Fn() + 'static,
) -> View {
    let mark = view()
        .size(px(12.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .border_1()
        .border_color(move || {
            if checked() {
                theme::ACCENT.get()
            } else {
                theme::TEXT_FAINT.get()
            }
        })
        .bg(move || {
            if checked() {
                theme::ACCENT.get()
            } else {
                theme::BACKGROUND.get()
            }
        })
        .text_size(px(9.0))
        .font_weight(FontWeight::BOLD)
        .text_color(theme::BACKGROUND)
        .child(text(move || if checked() { "✓" } else { "" }.to_owned()));
    view()
        .h(px(28.0))
        .w_full()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(10.0))
        .text_size(px(11.0))
        .text_color(theme::TEXT)
        .role(AccessibilityRole::Checkbox)
        .label(name)
        .disabled(disabled)
        .accessibility_state(move || AccessibilityState {
            checked: Some(if checked() {
                Checked::True
            } else {
                Checked::False
            }),
            disabled: disabled(),
            ..Default::default()
        })
        .hover(|style| style.bg(theme::SURFACE))
        .active(|style| style.bg(theme::BORDER))
        .disabled_style(|style| style.opacity(0.5))
        .on_press(on_toggle)
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .child(text(name).number_of_lines(1)),
        )
        .child(mark)
}
