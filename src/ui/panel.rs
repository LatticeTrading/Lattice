//! Movable floating panels: the drag that moves them, and the four panel
//! builders (below an anchor, centered, the old settings panel, a modal).

use nsh_core::prelude::*;

use crate::theme;

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
