//! The Settings panel, which the Everything Bar's gear opens: the user's
//! preferences, which are the theme and how the bar's clock reads.

use nsh_base::{SelectItem, select};
use nsh_core::prelude::*;

use super::clock::{self, Clock};
use crate::{
    theme::{self, ActiveTheme},
    ui::{self, Side, Tone},
};

const PANEL_WIDTH: Points = px(300.0);

/// The Settings panel, below the right end of `anchor`.
pub(super) fn panel(
    active: ActiveTheme,
    clock: Clock,
    anchor: &ElementRef,
    on_close: impl Fn() + Clone + 'static,
) -> Overlay {
    let body = view()
        .test_id("settings")
        .flex_col()
        .child(theme_row(active))
        .child(
            hour_cycle_row(clock)
                .border_t_1()
                .border_color(theme::BORDER),
        )
        .child(
            time_zone_row(clock)
                .border_t_1()
                .border_color(theme::BORDER),
        );
    ui::floating_panel(anchor, Side::Right, PANEL_WIDTH, "Settings", on_close, body)
}

/// "Theme", then the previous theme button, a dropdown of every theme
/// showing the current one, and the next theme button. The buttons wrap
/// around at either end of the list.
fn theme_row(active: ActiveTheme) -> View {
    let items: Vec<SelectItem> = theme::all()
        .iter()
        .map(|theme| SelectItem::new(theme.id, theme.name.clone()))
        .collect();
    let picker = dropdown(
        items,
        move || SharedString::from(active.get().id),
        move || active.get().name.clone(),
        move |id| active.select(id),
        "Theme",
    );
    row("Theme")
        .child(step(active, -1, "‹", "Previous theme"))
        .child(picker)
        .child(step(active, 1, "›", "Next theme"))
}

/// "Clock", then a choice of 24- or 12-hour time.
fn hour_cycle_row(clock: Clock) -> View {
    let choice = |label: &'static str, hour12: bool| {
        let selected = move || clock.get().hour12 == hour12;
        ui::button(label, Tone::Normal)
            .flex_1()
            .justify_center()
            .border_l_1()
            .border_color(theme::BORDER)
            .label(if hour12 {
                "12-hour clock"
            } else {
                "24-hour clock"
            })
            .accessibility_state(move || AccessibilityState {
                selected: selected(),
                ..Default::default()
            })
            .text_color(move || {
                if selected() {
                    theme::ACCENT.get()
                } else {
                    theme::TEXT_MUTED.get()
                }
            })
            .on_press(move || clock.update(|prefs| prefs.hour12 = hour12))
    };
    row("Clock")
        .child(choice("24h", false))
        .child(choice("12h", true))
}

/// "Time zone", then a dropdown of the clock's UTC offsets.
fn time_zone_row(clock: Clock) -> View {
    let items: Vec<SelectItem> = clock::OFFSETS
        .iter()
        .map(|&offset| SelectItem::new(offset.to_string(), clock::offset_label(offset)))
        .collect();
    let picker = dropdown(
        items,
        move || SharedString::from(clock.get().offset.to_string()),
        move || clock::offset_label(clock.get().offset),
        move |id| {
            if let Ok(offset) = id.parse::<i16>() {
                clock.update(|prefs| prefs.offset = offset);
            }
        },
        "Clock time zone",
    );
    row("Time zone").child(picker)
}

/// A settings row: its name, then the controls the caller adds.
fn row(name: &'static str) -> View {
    view()
        .h(ui::ROW_HEIGHT)
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

/// A dropdown filling the rest of a row: it shows `shown`, lists `items`,
/// marks the one whose id is `value`, and calls `on_pick` with a picked id.
fn dropdown(
    items: Vec<SelectItem>,
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
        .flex_1()
        .min_w(px(0.0))
        .h_full()
        .border_l_1()
        .border_color(theme::BORDER)
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
