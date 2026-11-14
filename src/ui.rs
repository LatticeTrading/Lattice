//! Shared Lattice controls: text fields, flat buttons, movable floating
//! panels and engine-driven list scrolling ([`RowScroll`]).
//!
//! NSH's `input` is a complete text field on every host (on Windows the
//! system's RichEdit text box, on the web an `<input>`), so typing, caret
//! movement, selection, undo, clipboard shortcuts, and IME composition need no
//! workarounds. A controlled `value` is safe while typing fast: the host drops
//! values based on an older edit. What these helpers add is Lattice's look and
//! one consistent way to bind a field to a signal, so the Layout Manager, the
//! symbol search, and future widgets all behave the same way.

mod row_scroll;

pub use row_scroll::RowScroll;

use nsh_base::{SelectItem, select};
use nsh_core::{
    Checked,
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
        // Windows' text box takes the selection as plain RGB, dropping
        // alpha, so the tint is laid over the field's background up front.
        .selection_color(|| {
            nsh_core::element::color(theme::BACKGROUND.get())
                .blend(nsh_core::element::color(theme::ACCENT.opacity(0.35).get()))
        })
        .auto_correct(false)
        .auto_capitalize(AutoCapitalize::None)
}

/// A settings row: its name, then the controls the caller adds.
pub fn setting_row(name: &'static str) -> View {
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

/// A row of the old app's settings panel (`.settings-row`): its name on
/// the left, on one line in primary text, and the control the caller adds
/// pushed to the right edge, 10pt in from both sides.
pub fn form_row(name: &'static str) -> View {
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

/// A [`form_row`] whose control is a list: its name level with the first
/// entry (the old `.settings-row-list`).
pub fn form_list_row(name: &'static str) -> View {
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

/// A dropdown filling the rest of a [`setting_row`]: it shows `shown`, lists `items`
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

/// A tooltip's box: `content` in small text on a panel (an element's
/// `.tooltip(..)`).
pub fn hint(content: impl Fn() -> String + 'static) -> View {
    view()
        .max_w(px(420.0))
        .px(px(6.0))
        .py(px(4.0))
        .bg(theme::PANEL)
        .border_1()
        .border_color(theme::BORDER)
        .text_size(px(10.0))
        .text_color(theme::TEXT)
        .child(text(content))
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
    fn new() -> Self {
        Self {
            origin: signal(None),
            start: signal(None),
        }
    }

    /// Places the panel where `opening` says, once what that needs is laid
    /// out; `panel` is the panel's own box, `width` its width. Called once
    /// the panel's view is made, so `panel` is on it.
    fn open(self, opening: Opening, panel: ElementRef, width: Points) {
        const INSET: f32 = 8.0;
        spawn(async move {
            let window = window();
            let origin = match opening {
                Opening::Below(anchor, side) => anchor.measure().await.map(|at| {
                    let x = match side {
                        Side::Left => at.page.x.0 + INSET,
                        Side::Right => at.page.x.0 + at.frame.size.width.0 - INSET - width.0,
                    };
                    // Kept inside the window: an anchor near its right edge
                    // would push a left-aligned panel past it.
                    let x = x.min(window.width.0 - width.0).max(0.0);
                    point(px(x), px(at.page.y.0 + at.frame.size.height.0))
                }),
                Opening::Centered => {
                    // The overlay is mounted after this task starts; until
                    // then the ref reaches nothing to measure.
                    for _ in 0..10 {
                        if panel.node().is_some() {
                            break;
                        }
                        next_frame().await;
                    }
                    panel.measure().await.map(|at| {
                        point(
                            px(((window.width.0 - at.frame.size.width.0) / 2.0).max(INSET)),
                            px(((window.height.0 - at.frame.size.height.0) / 2.0).max(INSET)),
                        )
                    })
                }
            };
            self.origin
                .set(Some(origin.unwrap_or(point(px(INSET), px(INSET)))));
        });
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

    /// Hidden until it is placed.
    fn placed(self) -> bool {
        self.origin.get().is_some()
    }
}

/// Where a floating panel opens.
#[derive(Clone, Copy)]
enum Opening {
    /// Below the anchor, lined up with its side edge.
    Below(ElementRef, Side),
    /// In the middle of the window.
    Centered,
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
    let opening = Opening::Below(*anchor, side);
    panel_overlay(opening, width, title, Look::Lattice, None, on_close, body)
}

/// A [`floating_panel`] that opens in the middle of the window instead of
/// below an anchor (the widget picker).
pub fn centered_panel(
    width: Points,
    title: &str,
    on_close: impl Fn() + Clone + 'static,
    body: impl IntoElement,
) -> Overlay {
    panel_overlay(
        Opening::Centered,
        width,
        title,
        Look::Lattice,
        None,
        on_close,
        body,
    )
}

/// A [`floating_panel`] that reads as the old app's widget settings panel
/// (`SettingsPanel.css`): a regular-weight title, and a close button that
/// turns red on hover.
pub fn settings_panel(
    anchor: &ElementRef,
    side: Side,
    width: Points,
    title: &str,
    on_close: impl Fn() + Clone + 'static,
    body: impl IntoElement,
) -> Overlay {
    let opening = Opening::Below(*anchor, side);
    let look = Look::Classic { title_inset: 10.0 };
    panel_overlay(opening, width, title, look, None, on_close, body)
}

/// A modal in the middle of the window, as the old app's symbol search was
/// (`SymbolSearch.css`): the window dimmed behind it ([`theme::BACKDROP`]),
/// a press on the dimmed part closing it, a regular-weight title with
/// `actions` (header buttons, before the close button) beside it. Two
/// overlays, the backdrop and then the panel above it, in a view that takes
/// no room where it is placed.
pub fn modal_panel(
    width: Points,
    title: &str,
    actions: Option<View>,
    on_close: impl Fn() + Clone + 'static,
    body: impl IntoElement,
) -> View {
    let backdrop = overlay()
        .inset_0()
        .child(view().size_full().bg(theme::BACKDROP).on_press({
            let on_close = on_close.clone();
            move || on_close()
        }));
    let look = Look::Classic { title_inset: 12.0 };
    let panel = panel_overlay(
        Opening::Centered,
        width,
        title,
        look,
        actions,
        on_close,
        body,
    );
    view().absolute().child(backdrop).child(panel)
}

/// How a floating panel's title bar reads.
#[derive(Clone, Copy, PartialEq)]
enum Look {
    /// Lattice's own panels (the Layout Manager, the widget picker): a bold
    /// title, a close button that lights up on hover.
    Lattice,
    /// The old app's settings panel and symbol search: a regular title
    /// `title_inset` from the edge, a close button that turns red.
    Classic { title_inset: f32 },
}

/// A floating panel's overlay, opening where `opening` says.
fn panel_overlay(
    opening: Opening,
    width: Points,
    title: &str,
    look: Look,
    actions: Option<View>,
    on_close: impl Fn() + Clone + 'static,
    body: impl IntoElement,
) -> Overlay {
    let drag = PanelDrag::new();
    let panel_ref = element_ref();
    let (inset, weight) = match look {
        Look::Lattice => (10.0, FontWeight::BOLD),
        Look::Classic { title_inset } => (title_inset, FontWeight::NORMAL),
    };

    // Everything left of the header's buttons grabs the panel.
    let grab = view()
        .h_full()
        .flex_1()
        .min_w(px(0.0))
        .flex()
        .items_center()
        .pl(px(inset))
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
        .hover(move |style| match look {
            Look::Lattice => style.bg(theme::SURFACE).text_color(theme::TEXT),
            Look::Classic { .. } => style.bg(theme::PANEL).text_color(theme::DANGER),
        })
        .active(|style| style.bg(theme::BORDER))
        .on_press({
            let on_close = on_close.clone();
            move || on_close()
        })
        .child(text("×"));

    let mut header = view()
        .h(px(40.0))
        .flex_none()
        .flex()
        .items_center()
        .bg(theme::BACKGROUND)
        .border_b_1()
        .border_color(theme::BORDER)
        .font_weight(weight)
        .text_size(px(12.0))
        .child(drag.handle(grab));
    if let Some(actions) = actions {
        header = header.child(actions);
    }
    let header = header.child(close);

    let panel = view()
        .element_ref(&panel_ref)
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
    let overlay = overlay()
        .left(move || drag.x())
        .top(move || drag.y())
        .opacity(move || if drag.placed() { 1.0_f32 } else { 0.0 })
        .on_cancel(on_close)
        .child(panel);
    drag.open(opening, panel_ref, width);
    overlay
}
