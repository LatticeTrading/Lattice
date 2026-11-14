//! The Settings panel, which the Everything Bar's gear opens: the user's
//! preferences, which are the theme and how the bar's clock reads.

use nsh_base::SelectItem;
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
    let picker = ui::dropdown(
        move || items.clone(),
        move || SharedString::from(active.get().id),
        move || active.get().name.clone(),
        move |id| active.select(id),
        "Theme",
    );
    ui::label_column_row("Theme")
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
    ui::label_column_row("Clock")
        .child(choice("24h", false))
        .child(choice("12h", true))
}

/// "Time zone", then a dropdown of the clock's UTC offsets.
fn time_zone_row(clock: Clock) -> View {
    let items: Vec<SelectItem> = clock::OFFSETS
        .iter()
        .map(|&offset| SelectItem::new(offset.to_string(), clock::offset_label(offset)))
        .collect();
    let picker = ui::dropdown(
        move || items.clone(),
        move || SharedString::from(clock.get().offset.to_string()),
        move || clock::offset_label(clock.get().offset),
        move |id| {
            if let Ok(offset) = id.parse::<i16>() {
                clock.update(|prefs| prefs.offset = offset);
            }
        },
        "Clock time zone",
    );
    ui::label_column_row("Time zone").child(picker)
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
