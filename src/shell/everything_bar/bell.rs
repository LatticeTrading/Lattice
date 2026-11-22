//! The notifications' cell of the bar (the bell) and the list that hangs
//! from it.
//!
//! - The bell stands between the connection status and Settings. A dot in
//!   the accent color says something was posted since the list was last
//!   open; its icon is the accent color while the list is open, as every
//!   cell of the bar that opens something.
//! - The list is everything the center keeps
//!   ([`Notifications`](crate::notify::Notifications)), newest first, each
//!   drawn as a popup draws it ([`block`]), over the row that clears it.
//!   Opening it is reading it: nothing is unread while it is open.
//!
//! Nothing of the notifications lives here but whether the list is open.

use nsh_core::{element::ImageSrc, prelude::*};

use super::icon_segment;
use crate::{
    notify::{Notifications, Posted},
    shell::{clock::Clock, notifications::block},
    theme,
    ui::{self, Side, Tone},
};

/// The unread dot's diameter.
const DOT: Points = px(6.0);
/// The box the bell is drawn in, whose top right corner the dot takes.
const ICON_BOX: Points = px(16.0);
/// The list's tallest, where the window has room.
const LIST_HEIGHT: f32 = 420.0;
/// What of the window's height is not the list's: the bar above the
/// panel, the panel's title bar and its Clear row, and room under it.
const AROUND_LIST: f32 = 160.0;

/// The bar's bell: the center, whether its list is open and what the list
/// hangs from.
#[derive(Clone, Copy)]
pub(super) struct Bell {
    center: Notifications,
    open: Signal<bool>,
    anchor: ElementRef,
}

impl Bell {
    pub(super) fn new(center: Notifications) -> Self {
        Self {
            center,
            open: signal(false),
            anchor: element_ref(),
        }
    }

    /// Whether the list shows (tracked).
    pub(super) fn is_open(self) -> bool {
        self.open.get()
    }

    /// The bar's cell, `width` wide.
    pub(super) fn button(self, width: f32) -> View {
        let Self { center, open, .. } = self;
        let unread = computed(move || center.unread());
        let icon = view()
            .relative()
            .size(ICON_BOX)
            .flex()
            .items_center()
            .justify_center()
            .pointer_events(PointerEvents::None)
            .child(
                svg(ImageSrc::svg_markup(include_str!(
                    "../../../assets/icons/bell.svg"
                )))
                .size(px(12.0)),
            )
            .child(
                view()
                    .absolute()
                    .top(px(0.0))
                    .right(px(0.0))
                    .size(DOT)
                    .rounded_full()
                    .bg(theme::ACCENT)
                    .style_when(move || unread.get() == 0, |style| style.hidden()),
            );
        icon_segment(width)
            .element_ref(&self.anchor)
            .test_id("notifications")
            .role(AccessibilityRole::Button)
            .label(move || match unread.get() {
                0 => "Notifications".to_owned(),
                unread => format!("Notifications: {unread} unread"),
            })
            .accessibility_state(move || AccessibilityState {
                expanded: Some(open.get()),
                has_popup: true,
                ..Default::default()
            })
            .text_color(move || {
                if open.get() {
                    theme::ACCENT.get()
                } else {
                    theme::TEXT_MUTED.get()
                }
            })
            .hover(|style| style.bg(theme::PANEL))
            .active(|style| style.bg(theme::BORDER))
            .on_press(move || open.update(|open| *open = !*open))
            .child(icon)
    }

    /// The list, below the bell and lined up with its right edge; its
    /// times are read on `clock`.
    pub(super) fn panel(self, clock: Clock) -> Overlay {
        let Self { center, open, .. } = self;
        // Opening it is reading it, and so is what arrives while it is
        // open.
        effect(move || {
            if center.unread() > 0 {
                center.mark_read();
            }
        });
        let empty = computed(move || center.is_empty());
        let nothing = view()
            .w_full()
            .px(px(10.0))
            .py(px(5.0))
            .border_b_1()
            .border_color(theme::BORDER)
            .text_size(px(11.0))
            .text_color(theme::TEXT_MUTED)
            .style_when(move || !empty.get(), |style| style.hidden())
            .child(text("No notifications yet."));
        let lines = each(
            move || center.history(),
            |posted: &Posted| posted.id,
            move |posted, _| {
                // The line is the list's; the level's edge the block's own.
                view()
                    .w_full()
                    .border_b_1()
                    .border_color(theme::BORDER)
                    .child(block(&posted.get_untracked(), clock))
            },
        );
        let list = scroll()
            .flex_none()
            .max_h(move || px((window().height.0 - AROUND_LIST).clamp(0.0, LIST_HEIGHT)))
            .child(view().w_full().flex_col().child(lines));
        let clear = ui::button("Clear all", Tone::Quiet)
            .h(ui::ROW_HEIGHT)
            .w_full()
            .label("Clear all notifications")
            .disabled(move || empty.get())
            .on_press(move || center.clear());
        let body = view()
            .test_id("notifications-panel")
            .flex_col()
            .child(nothing)
            .child(list)
            .child(clear);
        ui::floating_panel(
            &self.anchor,
            Side::Right,
            super::PANEL_WIDTH,
            "Notifications",
            move || open.set(false),
            body,
        )
    }
}
