//! The shell's side of the notifications ([`crate::notify`]):
//!
//! - [`block`] is how one is drawn, the same in the bell's list
//!   (`everything_bar/bell.rs`) and in a popup: an edge in the color of
//!   how much it matters, its title with who says it and when at the far
//!   edge, and the particulars under it;
//! - [`popups`] are the ones popped up now, in the corner of the window
//!   the user chose: one bordered stack of blocks, the newest nearest the
//!   corner, each with a × that takes it down. One goes by itself after
//!   [`POPUP_FOR`], unless the pointer rests on it;
//! - [`fills`] is what the shell itself posts: the trades of the
//!   connected wallet's account.

pub(super) mod fills;
#[cfg(test)]
mod tests;

use std::time::Duration;

use nsh_core::{LineHeight, prelude::*};

use super::{
    clock::{self, Clock},
    everything_bar::BAR_HEIGHT,
};
use crate::{
    notify::{Level, Notifications, Posted},
    theme,
    ui::{self, Tone},
};

/// How wide the popups are, where the window has room.
const POPUP_WIDTH: f32 = 320.0;
/// How far the popups keep from the window's edges: the shell's padding,
/// and as much again inside the panes.
const EDGE: f32 = 16.0;
/// Where the popups of a top corner begin: under the Everything Bar (the
/// shell's padding, the bar, and the space between panes).
const UNDER_BAR: f32 = 8.0 + BAR_HEIGHT.0 + 8.0;
/// How long a popup shows before it goes by itself.
const POPUP_FOR: Duration = Duration::from_secs(6);
/// The width of the level's edge, as a strip's.
const LEVEL_EDGE: Points = px(3.0);

/// The color of `level`'s edge.
const fn edge(level: Level) -> theme::Role {
    match level {
        Level::Info => theme::TEXT_FAINT,
        Level::Success => theme::SUCCESS,
        Level::Warning => theme::WARNING,
        Level::Danger => theme::DANGER,
    }
}

/// A note at the far edge of a notification's title: who says it, when.
fn note(content: impl IntoReactive<SharedString>) -> Text {
    text(content)
        .flex_none()
        .number_of_lines(1)
        .text_size(px(10.0))
        .text_color(theme::TEXT_FAINT)
}

/// One notification: `posted` as the list and a popup both draw it, its
/// time read on `clock`. It has no line of its own but the level's edge:
/// what holds it draws the seams.
pub(super) fn block(posted: &Posted, clock: Clock) -> View {
    let notice = &posted.notice;
    let at = (posted.at_ms / 1_000.0).floor() as i64;
    let head = view()
        .w_full()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .child(
            text(notice.title.clone())
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::TEXT),
        )
        .children(notice.origin.clone().map(note))
        .child(note(move || clock::format_time(at, clock.get())).tabular_nums());
    let body = notice.body.clone().map(|body| {
        text(body)
            .text_color(theme::TEXT_MUTED)
            .line_height(LineHeight::Multiple(1.5))
    });
    view()
        .flex_col()
        .gap(px(2.0))
        .px(px(10.0))
        .py(px(6.0))
        .border_l(LEVEL_EDGE)
        .border_l_color(edge(notice.level))
        .text_size(px(11.0))
        .role(if notice.level == Level::Danger {
            AccessibilityRole::Alert
        } else {
            AccessibilityRole::Text
        })
        .child(head)
        .children(body)
}

/// The popups, in the user's corner of the window: made when the first
/// pops up and gone with the last, so they lie over whatever was open
/// before them.
pub(super) fn popups(center: Notifications, clock: Clock) -> impl IntoElement {
    let any = computed(move || center.has_popups());
    let corner = computed(move || center.prefs().corner);
    dynamic(move || {
        if !any.get() {
            return None;
        }
        let corner = corner.get();
        let stack = view()
            .test_id("notification-popups")
            .w(move || px((window().width.0 - 2.0 * EDGE).clamp(0.0, POPUP_WIDTH)))
            .flex_col()
            .bg(theme::PANEL)
            .border_1()
            .border_color(theme::BORDER)
            .text_color(theme::TEXT)
            .child(each(
                // The newest nearest the corner.
                move || {
                    let mut popups = center.popups();
                    if corner.is_top() {
                        popups.reverse();
                    }
                    popups
                },
                |posted: &Posted| posted.id,
                move |posted, index| popup(center, clock, &posted.get_untracked(), index),
            ));
        // In an overlay, so it takes its presses over the panes
        // (`ui::floating_panel` says why); with no anchor it is a fixed
        // box of the window.
        let placed = overlay().child(stack);
        let placed = if corner.is_left() {
            placed.left(px(EDGE))
        } else {
            placed.right(px(EDGE))
        };
        Some(if corner.is_top() {
            placed.top(px(UNDER_BAR))
        } else {
            placed.bottom(px(EDGE))
        })
    })
}

/// One popup of the stack: the notification, then the × that takes it
/// down.
fn popup(center: Notifications, clock: Clock, posted: &Posted, index: ReadSignal<usize>) -> View {
    let id = posted.id;
    // It goes by itself after a while, counted again from when the pointer
    // leaves it: one the pointer rests on is being read.
    let over = signal(false);
    effect(move || {
        if over.get() {
            return;
        }
        let going = set_timeout(POPUP_FOR, move || center.dismiss(id));
        cleanup(move || going.cancel());
    });
    view()
        .test_id(&format!("notification-popup-{id}"))
        .w_full()
        .flex_row()
        .style_when(
            move || index.get() > 0,
            |style| style.border_t_1().border_color(theme::BORDER),
        )
        .on_mouse_enter(move |_: &MouseEvent| over.set(true))
        .on_mouse_leave(move |_: &MouseEvent| over.set(false))
        .child(block(posted, clock).flex_1().min_w(px(0.0)))
        .child(
            ui::button("×", Tone::Quiet)
                .w(ui::ROW_HEIGHT)
                .flex_none()
                .justify_center()
                .border_l_1()
                .border_color(theme::BORDER)
                .label("Dismiss notification")
                .test_id(&format!("notification-dismiss-{id}"))
                .on_press(move || center.dismiss(id)),
        )
}
