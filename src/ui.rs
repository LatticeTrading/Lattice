//! Shared Lattice controls: text fields, flat buttons, and movable floating
//! panels.
//!
//! NSH's `input` is a complete text field on every host (on Windows NSH's own
//! Direct2D editor, on the web an `<input>`), so typing, caret movement,
//! selection, undo, clipboard shortcuts, and IME composition need no
//! workarounds. A controlled `value` is safe while typing fast: the host drops
//! values based on an older edit. What these helpers add is Lattice's look and
//! one consistent way to bind a field to a signal, so the Layout Manager, the
//! symbol search, and future widgets all behave the same way.

use nsh_core::{
    element::{Input, input},
    prelude::*,
};

use crate::theme;

/// The default height of a single-line field or a panel row.
pub const ROW_HEIGHT: Points = px(36.0);

/// A single-line text field bound to `value`.
///
/// The field is controlled: every edit writes `value`, and writing `value`
/// from code (clearing it, filling a generated key) updates the field.
/// Chain `placeholder`, `label`, `on_submit`, or more `on_change` listeners
/// onto the returned `Input` as needed.
pub fn text_field(value: Signal<String>) -> Input {
    input()
        .value(move || value.get())
        .on_change(move |text: &str| value.set(text.to_owned()))
        .h(ROW_HEIGHT)
        .min_w(px(0.0))
        .px(px(10.0))
        .text_size(px(11.0))
        .text_color(theme::TEXT)
        .bg(theme::BACKGROUND)
        .placeholder_color(theme::TEXT_FAINT)
        .caret_color(theme::ACCENT)
        .selection_color(theme::ACCENT.opacity(0.35))
        .auto_correct(false)
        .auto_capitalize(AutoCapitalize::None)
}

/// How a [`button`] reads.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Primary text: ordinary actions.
    Normal,
    /// Muted text that turns red on hover: a destructive row action.
    Quiet,
    /// Red text: confirming something destructive.
    Danger,
}

/// A flat, borderless text button that fills its row's height, with hover,
/// press, focus, and disabled states.
pub fn button(label: impl Into<String>, tone: Tone) -> View {
    let color = match tone {
        Tone::Normal => theme::TEXT,
        Tone::Quiet => theme::TEXT_MUTED,
        Tone::Danger => theme::DANGER,
    };
    view()
        .h_full()
        .flex()
        .items_center()
        .px(px(10.0))
        .text_size(px(11.0))
        .text_color(color)
        .role(AccessibilityRole::Button)
        .hover(move |style| {
            let style = style.bg(theme::SURFACE);
            if tone == Tone::Quiet {
                style.text_color(theme::DANGER)
            } else {
                style
            }
        })
        .active(|style| style.bg(theme::BORDER))
        .disabled_style(|style| style.opacity(0.5))
        .child(text(label.into()))
}

/// Where a floating panel is in the window and the drag moving it.
///
/// Made when the panel is shown, so every opening starts back at its anchor.
/// The overlay itself is moved (its `left`/`top`), not transformed: NSH
/// hit-tests an overlay where it is laid out, so a translated panel would
/// stop taking presses, and further drags, once it left its first spot.
#[derive(Clone, Copy)]
struct PanelDrag {
    /// The panel's top left corner in the window; `None` until the anchor
    /// has been measured.
    origin: Signal<Option<Point<Points>>>,
    /// The origin and pointer position when the current drag began.
    start: Signal<Option<(Point<Points>, Point<Points>)>>,
}

impl PanelDrag {
    /// Opens below `anchor`, `inset` in from its `side` edge, for a panel
    /// `width` wide.
    fn below(anchor: ElementRef, side: Side, width: Points, inset: Points) -> Self {
        let drag = Self {
            origin: signal(None),
            start: signal(None),
        };
        spawn(async move {
            let origin = match anchor.measure().await {
                Some(at) => {
                    let x = match side {
                        Side::Left => at.page.x.0 + inset.0,
                        Side::Right => at.page.x.0 + at.frame.size.width.0 - inset.0 - width.0,
                    };
                    point(px(x.max(0.0)), px(at.page.y.0 + at.frame.size.height.0))
                }
                None => point(inset, inset),
            };
            drag.origin.set(Some(origin));
        });
        drag
    }

    /// Makes `handle` move the panel. The pointer is kept inside the window,
    /// so the part of the panel being held never leaves it and the panel can
    /// always be dragged back.
    fn handle(self, handle: View) -> View {
        handle
            .cursor(Cursor::Move)
            .claim_touches(TouchClaim::on_start())
            .keep_touches(true)
            .on_responder_grant(move |event: &ResponderEvent| {
                if let Some(origin) = self.origin.get_untracked() {
                    self.start.set(Some((origin, event.page)));
                }
            })
            .on_responder_move(move |event: &ResponderEvent| {
                let Some((origin, from)) = self.start.get_untracked() else {
                    return;
                };
                let window = window();
                let to = point(
                    px(event.page.x.0.clamp(0.0, window.width.0)),
                    px(event.page.y.0.clamp(0.0, window.height.0)),
                );
                self.origin.set(Some(point(
                    px(origin.x.0 + to.x.0 - from.x.0),
                    px(origin.y.0 + to.y.0 - from.y.0),
                )));
            })
            .on_responder_release(move |_: &ResponderEvent| self.start.set(None))
            .on_responder_terminate(move |_: &ResponderEvent| self.start.set(None))
    }

    fn x(self) -> Points {
        self.origin.get().map_or(px(0.0), |origin| origin.x)
    }

    fn y(self) -> Points {
        self.origin.get().map_or(px(0.0), |origin| origin.y)
    }

    /// Hidden for the frame before the anchor is measured.
    fn placed(self) -> bool {
        self.origin.get().is_some()
    }
}

/// Which edge of its anchor a [`floating_panel`] opens against.
#[derive(Clone, Copy)]
pub enum Side {
    Left,
    Right,
}

/// A movable floating panel (the Layout Manager, Settings, pickers): a
/// title bar that drags it and closes it, above `body`, `width` wide. It
/// opens below `anchor`, lined up with its `side` edge.
///
/// Rendering it inside an [`overlay`] is what makes its controls clickable:
/// an absolutely positioned child that hangs outside its parent is drawn and
/// hit-tested in its parent's slot, so whatever comes later in the tree (the
/// workspace grid, under the Everything Bar) takes its clicks. The overlay
/// lifts it into the window's top layer. Escape and the close button call
/// `on_close`; leaving the overlay out of the view is the caller's job.
pub fn floating_panel(
    anchor: &ElementRef,
    side: Side,
    width: Points,
    title: &str,
    on_close: impl Fn() + Clone + 'static,
    body: impl IntoElement,
) -> Overlay {
    let drag = PanelDrag::below(*anchor, side, width, px(8.0));

    // Everything left of the close button grabs the panel.
    let grab = view()
        .h_full()
        .flex_1()
        .min_w(px(0.0))
        .flex()
        .items_center()
        .pl(px(10.0))
        .label(format!("Move {title}"))
        .child(text(title.to_owned()).pointer_events(PointerEvents::None));

    let close = view()
        .h_full()
        .w(px(40.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .border_l_1()
        .border_color(theme::BORDER)
        .text_color(theme::TEXT_FAINT)
        .role(AccessibilityRole::Button)
        .label(format!("Close {title}"))
        .hover(|style| style.bg(theme::SURFACE).text_color(theme::TEXT))
        .active(|style| style.bg(theme::BORDER))
        .on_press({
            let on_close = on_close.clone();
            move || on_close()
        })
        .child(text("×"));

    let header = view()
        .h(px(40.0))
        .flex_none()
        .flex()
        .items_center()
        .bg(theme::BACKGROUND)
        .border_b_1()
        .border_color(theme::BORDER)
        .font_weight(FontWeight::BOLD)
        .text_size(px(12.0))
        .child(drag.handle(grab))
        .child(close);

    let panel = view()
        .label(title.to_owned())
        .w(width)
        .flex_col()
        .border_1()
        .border_color(theme::BORDER)
        .bg(theme::PANEL)
        .text_color(theme::TEXT)
        .child(header)
        .child(body);

    // Without an anchor an overlay is a `position: fixed` box of the window.
    overlay()
        .left(move || drag.x())
        .top(move || drag.y())
        .opacity(move || if drag.placed() { 1.0_f32 } else { 0.0 })
        .on_cancel(on_close)
        .child(panel)
}
