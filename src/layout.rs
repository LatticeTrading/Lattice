//! First migration checkpoint: a scrollable grid of blank, resizable mock panes.
mod grid;

use crate::theme::{self, color};
use gpui_kit::{
    AppContext, Context, DragMoveEvent, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Render, ScrollHandle,
    StatefulInteractiveElement, Styled, Subscription, TestSupportExt, Window, canvas, div, px,
};
use grid::{
    GAP, Grid, Pane, ROW_STEP, Rect, breakpoint, column_step, columns, compact, moved, resized,
};

struct ResizeGhost;

#[derive(Clone, Copy)]
struct ResizeDrag {
    id: usize,
    breakpoint: usize,
}

#[derive(Clone, Copy)]
struct MoveDrag {
    id: usize,
    breakpoint: usize,
}

impl Render for ResizeGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

struct Resize {
    id: usize,
    breakpoint: usize,
    start: Point<Pixels>,
    scroll_y: Pixels,
    before: Vec<Pane>,
    preview: Rect,
}

struct Move {
    id: usize,
    breakpoint: usize,
    start: Point<Pixels>,
    scroll_y: Pixels,
    before: Vec<Pane>,
    preview: Rect,
}

pub struct Workspace {
    grid: Grid,
    resize: Option<Resize>,
    pane_move: Option<Move>,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.cancel(cx);
                }
            }),
            cx.observe_window_bounds(window, |this, _, cx| this.cancel(cx)),
        ];
        Self {
            grid: Grid::default(),
            resize: None,
            pane_move: None,
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn spawn(&mut self, cx: &mut Context<Self>) {
        self.cancel(cx);
        self.grid.spawn();
        cx.notify();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if let Some(resize) = self.resize.take() {
            self.grid.layouts[resize.breakpoint] = resize.before;
            cx.notify();
        }
        if let Some(pane_move) = self.pane_move.take() {
            self.grid.layouts[pane_move.breakpoint] = pane_move.before;
            cx.notify();
        }
    }

    fn move_to(&mut self, position: Point<Pixels>, width: f32, cx: &mut Context<Self>) {
        let Some(pane_move) = self.pane_move.as_ref() else {
            return;
        };
        let breakpoint = pane_move.breakpoint;
        let id = pane_move.id;
        let original = pane_move
            .before
            .iter()
            .find(|pane| pane.id == id)
            .expect("move target")
            .rect;
        let preview = moved(
            original,
            f32::from(position.x - pane_move.start.x),
            f32::from(position.y - pane_move.start.y + pane_move.scroll_y - self.scroll.offset().y),
            width,
            columns(breakpoint),
        );
        if pane_move.preview == preview {
            return;
        }
        let mut layout = pane_move.before.clone();
        if let Some(pane) = layout.iter_mut().find(|pane| pane.id == id) {
            pane.rect = preview;
        }
        compact(&mut layout, Some(id));
        self.grid.layouts[breakpoint] = layout;
        if let Some(pane_move) = &mut self.pane_move {
            pane_move.preview = preview;
        }
        cx.notify();
    }

    fn finish_move(&mut self, cx: &mut Context<Self>) {
        if self.pane_move.take().is_some() {
            cx.notify();
        }
    }

    fn resize_to(&mut self, position: Point<Pixels>, width: f32, cx: &mut Context<Self>) {
        let Some(resize) = self.resize.as_ref() else {
            return;
        };
        let breakpoint = resize.breakpoint;
        let id = resize.id;
        let original = resize
            .before
            .iter()
            .find(|pane| pane.id == id)
            .expect("resize target")
            .rect;
        let preview = resized(
            original,
            f32::from(position.x - resize.start.x),
            f32::from(position.y - resize.start.y + resize.scroll_y - self.scroll.offset().y),
            width,
            columns(breakpoint),
        );
        if self
            .resize
            .as_ref()
            .is_some_and(|resize| resize.preview == preview)
        {
            return;
        }

        // Rebuild from the original drag snapshot so live updates remain
        // deterministic and never compound rounding or collision movement.
        let mut layout = self.resize.as_ref().expect("resize state").before.clone();
        if let Some(pane) = layout.iter_mut().find(|pane| pane.id == id) {
            pane.rect = preview;
        }
        compact(&mut layout, Some(id));
        self.grid.layouts[breakpoint] = layout;
        if let Some(resize) = &mut self.resize {
            resize.preview = preview;
        }
        // This fires only after a snapped grid cell changes, not for every
        // pointer event, keeping live widget relayout bounded and predictable.
        cx.notify();
    }

    fn finish_resize(&mut self, cx: &mut Context<Self>) {
        let Some(resize) = self.resize.take() else {
            return;
        };
        let mut layout = resize.before;
        if let Some(pane) = layout.iter_mut().find(|pane| pane.id == resize.id) {
            pane.rect = resize.preview;
        }
        compact(&mut layout, Some(resize.id));
        self.grid.layouts[resize.breakpoint] = layout;
        cx.notify();
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Shell owns the 8px horizontal inset; grid width excludes that chrome.
        let width = f32::from(window.viewport_size().width) - 16.;
        let bp = breakpoint(width);
        let step = column_step(width, columns(bp));
        let layout = &self.grid.layouts[bp];
        let height = layout
            .iter()
            .map(|pane| pane.rect.y + pane.rect.h)
            .max()
            .unwrap_or(0) as f32
            * ROW_STEP
            + GAP;
        let panes = layout
            .iter()
            .map(|pane| {
                let id = pane.id;
                let rect = pane.rect;
                let tabs = pane.tabs.clone();
                let active_tab = pane.active_tab;
                div()
                    .id(("pane", id))
                    .test_support()
                    .relative()
                    .absolute()
                    .left(px(rect.x as f32 * step))
                    .top(px(GAP + rect.y as f32 * ROW_STEP))
                    .w(px(rect.w as f32 * step - GAP))
                    .h(px(rect.h as f32 * ROW_STEP - GAP))
                    .border_1()
                    .border_color(color(theme::BORDER))
                    .bg(color(theme::BAR_BACKGROUND))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id(("pane-header", id))
                            .h(px(32.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .bg(color(theme::BAR_BACKGROUND))
                            .border_b_1()
                            .border_color(color(theme::BORDER))
                            .children(tabs.iter().map(|tab| {
                                let tab_id = tab.id;
                                let selected = tab_id == active_tab;
                                div()
                                    .id(format!("tab-{id}-{tab_id}"))
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .px(px(12.))
                                    .text_size(px(11.))
                                    .text_color(color(if selected {
                                        theme::TEXT_PRIMARY
                                    } else {
                                        theme::TEXT_SECONDARY
                                    }))
                                    .bg(color(if selected {
                                        theme::WINDOW_BACKGROUND
                                    } else {
                                        theme::BAR_BACKGROUND
                                    }))
                                    .border_r_1()
                                    .border_color(color(theme::BORDER))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.grid.activate_tab(id, tab_id);
                                        cx.notify();
                                    }))
                                    .child(tab.title.clone())
                                    .child(
                                        div()
                                            .id(format!("tab-close-{id}-{tab_id}"))
                                            .text_color(color(theme::TEXT_TERTIARY))
                                            .cursor_pointer()
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.grid.close_tab(id, tab_id);
                                                cx.stop_propagation();
                                                cx.notify();
                                            }))
                                            .child("×"),
                                    )
                            }))
                            .child(
                                div()
                                    .id(("tab-add", id))
                                    .h_full()
                                    .w(px(32.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(14.))
                                    .text_color(color(theme::TEXT_SECONDARY))
                                    .border_r_1()
                                    .border_color(color(theme::BORDER))
                                    .cursor_pointer()
                                    .hover(|this| this.bg(color(theme::WINDOW_BACKGROUND)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.grid.add_tab(id);
                                        cx.notify();
                                    }))
                                    .child("+"),
                            )
                            .child(
                                div()
                                    .id(("pane-drag", id))
                                    .aria_label("Move pane")
                                    .h_full()
                                    .w(px(32.))
                                    .flex_none()
                                    .ml_auto()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_color(color(theme::TEXT_TERTIARY))
                                    .border_l_1()
                                    .border_color(color(theme::BORDER))
                                    .cursor_move()
                                    .on_drag(MoveDrag { id, breakpoint: bp }, |_, _, _, cx| {
                                        cx.new(|_| ResizeGhost)
                                    })
                                    .child("⠿"),
                            )
                            .child(
                                div()
                                    .id(("pane-close", id))
                                    .aria_label("Close pane")
                                    .h_full()
                                    .w(px(32.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(14.))
                                    .text_color(color(theme::TEXT_TERTIARY))
                                    .border_l_1()
                                    .border_color(color(theme::BORDER))
                                    .cursor_pointer()
                                    .hover(|this| {
                                        this.bg(color(theme::WINDOW_BACKGROUND))
                                            .text_color(color(theme::TEXT_PRIMARY))
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.grid.remove_pane(id);
                                        cx.stop_propagation();
                                        cx.notify();
                                    }))
                                    .child("×"),
                            ),
                    )
                    .child(
                        div()
                            .id(("pane-body", id))
                            .flex_1()
                            .min_h_0()
                            .bg(color(theme::WINDOW_BACKGROUND))
                            .child(div().flex_1().size_full()),
                    )
                    .child(
                        div()
                            .id(("resize-pane", id))
                            .aria_label("Resize blank pane")
                            .test_support()
                            .absolute()
                            .right_0()
                            .bottom_0()
                            .size(px(18.))
                            .cursor_nwse_resize()
                            .flex()
                            .items_end()
                            .justify_end()
                            .p(px(3.))
                            .on_drag(ResizeDrag { id, breakpoint: bp }, |_, _, _, cx| {
                                cx.new(|_| ResizeGhost)
                            })
                            .child(
                                div()
                                    .size(px(7.))
                                    .border_r_1()
                                    .border_b_1()
                                    .border_color(color(theme::TEXT_TERTIARY)),
                            ),
                    )
            })
            .collect::<Vec<_>>();
        let entity = cx.entity().downgrade();
        div()
            .id("workspace")
            .test_support()
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<ResizeDrag>, window, cx| {
                    let drag = event.drag(cx);
                    let id = drag.id;
                    let bp = drag.breakpoint;
                    if this.resize.is_none() {
                        let preview = this.grid.layouts[bp]
                            .iter()
                            .find(|pane| pane.id == id)
                            .expect("resize target")
                            .rect;
                        this.resize = Some(Resize {
                            id,
                            breakpoint: bp,
                            start: event.event.position,
                            scroll_y: this.scroll.offset().y,
                            before: this.grid.layouts[bp].clone(),
                            preview,
                        });
                    }
                    this.resize_to(
                        event.event.position,
                        f32::from(window.viewport_size().width) - 16.,
                        cx,
                    );
                    cx.stop_propagation();
                }),
            )
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<MoveDrag>, window, cx| {
                    let drag = event.drag(cx);
                    let id = drag.id;
                    let bp = drag.breakpoint;
                    if this.pane_move.is_none() {
                        let preview = this.grid.layouts[bp]
                            .iter()
                            .find(|pane| pane.id == id)
                            .expect("move target")
                            .rect;
                        this.pane_move = Some(Move {
                            id,
                            breakpoint: bp,
                            start: event.event.position,
                            scroll_y: this.scroll.offset().y,
                            before: this.grid.layouts[bp].clone(),
                            preview,
                        });
                    }
                    this.move_to(
                        event.event.position,
                        f32::from(window.viewport_size().width) - 16.,
                        cx,
                    );
                    cx.stop_propagation();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.finish_resize(cx);
                    this.finish_move(cx);
                }),
            )
            .child(div().relative().w_full().h(px(height)).children(panes))
            // Paint-scoped listeners continue the gesture outside the small handle,
            // including over other panes. No native desktop automation/capture.
            .child(
                canvas(
                    |_, _, _| (),
                    move |_, _, window, _| {
                        let owner = entity.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                            if phase.capture() {
                                let _ = owner.update(cx, |this, cx| {
                                    if !event.dragging() {
                                        this.cancel(cx);
                                    } else {
                                        this.resize_to(
                                            event.position,
                                            f32::from(window.viewport_size().width) - 16.,
                                            cx,
                                        );
                                    }
                                });
                            }
                        });
                        let owner = entity.clone();
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                            if phase.capture() && event.button == MouseButton::Left {
                                let _ = owner.update(cx, |this, cx| {
                                    if this.resize.is_some() {
                                        this.resize_to(
                                            event.position,
                                            f32::from(window.viewport_size().width) - 16.,
                                            cx,
                                        );
                                        this.finish_resize(cx);
                                        cx.stop_propagation();
                                    } else if this.pane_move.is_some() {
                                        this.move_to(
                                            event.position,
                                            f32::from(window.viewport_size().width) - 16.,
                                            cx,
                                        );
                                        this.finish_move(cx);
                                        cx.stop_propagation();
                                    }
                                });
                            }
                        });
                        let owner = entity.clone();
                        window.on_key_event(move |event: &KeyDownEvent, phase, _, cx| {
                            if phase.capture() && event.keystroke.key == "escape" {
                                let _ = owner.update(cx, |this, cx| this.cancel(cx));
                            }
                        });
                    },
                )
                .absolute()
                .size_0(),
            )
    }
}
