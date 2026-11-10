//! The shared workspace/grid view and its persisted page operations.
//!
//! Grid geometry remains in `layout/grid.rs`; this module adapts that model to
//! NSH elements and signals. The same component tree is used by desktop and
//! WebAssembly runners.

mod grid;
#[cfg(test)]
mod tests;
pub(crate) use grid::Grid;

use crate::{
    storage::{self, StoredPage, StoredWorkspace},
    theme,
};
use grid::{GAP, Pane, ROW_STEP, breakpoint, column_step, columns};
use nsh_core::prelude::*;

#[derive(Clone, PartialEq)]
struct DragState {
    id: usize,
    page_id: usize,
    breakpoint: usize,
    width: f32,
    before: Vec<Pane>,
    preview: Vec<Pane>,
    resize: bool,
}

impl DragState {
    fn valid(&self, saved: &StoredWorkspace, width: f32) -> bool {
        saved.active_page_id == self.page_id
            && width == self.width
            && active_grid(saved).layouts[self.breakpoint] == self.before
    }

    fn update(&mut self, dx: f32, dy: f32) {
        self.preview = grid::dragged(
            &self.before,
            self.id,
            dx,
            dy,
            self.width,
            self.breakpoint,
            self.resize,
        );
    }

    fn commit(self, saved: &mut StoredWorkspace, width: f32) -> bool {
        if !self.valid(saved, width) || self.preview == self.before {
            return false;
        }
        active_grid_mut(saved).layouts[self.breakpoint] = self.preview;
        true
    }
}

fn workspace_width() -> f32 {
    (window().width.0 - 16.0).max(1.0)
}

/// Repair a loaded workspace and ensure it always has one real page.
pub(crate) fn normalize(workspace: &mut StoredWorkspace) {
    if workspace.pages.is_empty() {
        workspace.pages.push(StoredPage {
            id: workspace.active_page_id.max(1),
            grid: Grid::default(),
        });
    }
    if !workspace
        .pages
        .iter()
        .any(|page| page.id == workspace.active_page_id)
    {
        workspace.active_page_id = workspace.pages[0].id;
    }
    workspace.next_page_id = workspace
        .pages
        .iter()
        .map(|page| page.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
        .max(workspace.next_page_id);
    for page in &mut workspace.pages {
        page.grid.repair_counters();
    }
}

fn active_grid(workspace: &StoredWorkspace) -> &Grid {
    &workspace
        .pages
        .iter()
        .find(|page| page.id == workspace.active_page_id)
        .expect("workspace has an active page")
        .grid
}

pub(crate) fn select_page(workspace: &mut StoredWorkspace, page_id: usize) {
    if workspace.pages.iter().any(|page| page.id == page_id) {
        workspace.active_page_id = page_id;
    }
}

pub(crate) fn add_page(workspace: &mut StoredWorkspace) {
    let id = workspace.next_page_id;
    workspace.next_page_id = workspace.next_page_id.saturating_add(1);
    workspace.pages.push(StoredPage {
        id,
        grid: Grid::default(),
    });
    workspace.active_page_id = id;
}

pub(crate) fn spawn(workspace: &mut StoredWorkspace) {
    active_grid_mut(workspace).spawn();
}

fn active_grid_mut(workspace: &mut StoredWorkspace) -> &mut Grid {
    &mut workspace
        .pages
        .iter_mut()
        .find(|page| page.id == workspace.active_page_id)
        .expect("workspace has an active page")
        .grid
}

pub(crate) fn remove_pane(workspace: &mut StoredWorkspace, pane_id: usize) {
    active_grid_mut(workspace).remove_pane(pane_id);
}

pub(crate) fn add_tab(workspace: &mut StoredWorkspace, pane_id: usize) {
    active_grid_mut(workspace).add_tab(pane_id);
}

pub(crate) fn activate_tab(workspace: &mut StoredWorkspace, pane_id: usize, tab_id: usize) {
    active_grid_mut(workspace).activate_tab(pane_id, tab_id);
}

pub(crate) fn close_tab(workspace: &mut StoredWorkspace, pane_id: usize, tab_id: usize) {
    active_grid_mut(workspace).close_tab(pane_id, tab_id);
}

/// NSH's touch responder also carries desktop mouse drags, including moves
/// outside the handle. Share the same lifecycle for move and resize controls.
fn drag_handle(
    handle: View,
    workspace: Signal<StoredWorkspace>,
    drag: Signal<Option<DragState>>,
    id: usize,
    resize: bool,
) -> View {
    handle
        .claim_touches(TouchClaim::on_start())
        .keep_touches(true)
        .on_responder_grant(move |_: &ResponderEvent| {
            let saved = workspace.get_untracked();
            let width = workspace_width();
            let breakpoint = breakpoint(width);
            let before = active_grid(&saved).layouts[breakpoint].clone();
            if before.iter().any(|pane| pane.id == id) {
                drag.set(Some(DragState {
                    id,
                    page_id: saved.active_page_id,
                    breakpoint,
                    width,
                    preview: before.clone(),
                    before,
                    resize,
                }));
            }
        })
        .on_responder_move(move |event: &ResponderEvent| {
            let Some(mut state) = drag.get_untracked() else {
                return;
            };
            if !state.valid(&workspace.get_untracked(), workspace_width()) {
                drag.set(None);
                return;
            }
            state.update(event.gesture.delta.x.0, event.gesture.delta.y.0);
            // Signal::set compares values: sub-cell moves do not redraw.
            drag.set(Some(state));
        })
        .on_responder_release(move |_: &ResponderEvent| {
            let Some(state) = drag.get_untracked() else {
                return;
            };
            batch(|| {
                drag.set(None);
                let mut saved = workspace.get_untracked();
                if state.commit(&mut saved, workspace_width()) {
                    persist(&saved);
                    workspace.set(saved);
                }
            });
        })
        .on_responder_terminate(move |_: &ResponderEvent| drag.set(None))
}

fn persist(workspace: &StoredWorkspace) {
    if let Err(error) = storage::save(workspace) {
        log::warn!("Lattice: unable to save workspace: {error}");
    }
}

/// Render the active responsive layout.
pub(crate) fn workspace(workspace: Signal<StoredWorkspace>) -> impl IntoElement {
    let drag_state = signal(None::<DragState>);
    // Invalidate rather than restoring an old snapshot over newer page/tab
    // changes. Previews never enter the persisted workspace until release.
    effect(move || {
        let saved = workspace.get();
        let width = workspace_width();
        if drag_state
            .get_untracked()
            .is_some_and(|drag| !drag.valid(&saved, width))
        {
            drag_state.set(None);
        }
    });

    dynamic(move || {
        let saved = workspace.get();
        let width = workspace_width();
        let bp = breakpoint(width);
        let cols = columns(bp);
        let step = column_step(width, cols);
        let panes = drag_state
            .get()
            .filter(|drag| drag.valid(&saved, width))
            .map(|drag| drag.preview)
            .unwrap_or_else(|| active_grid(&saved).layouts[bp].clone());
        let height = panes
            .iter()
            .map(|pane| pane.rect.y + pane.rect.h)
            .max()
            .unwrap_or(0) as f32
            * ROW_STEP
            + GAP;

        let rendered = panes
            .into_iter()
            .map(|pane| {
                let id = pane.id;
                let rect = pane.rect;
                let active_tab = pane.active_tab;
                let tabs = pane.tabs;
                let tab_views = tabs
                    .into_iter()
                    .map(|tab| {
                        let tab_id = tab.id;
                        let selected = tab_id == active_tab;
                        let title = tab.title;
                        view()
                            .h_full()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .px(px(12.0))
                            .text_size(px(11.0))
                            .text_color(if selected {
                                theme::foreground()
                            } else {
                                theme::secondary()
                            })
                            .bg(if selected {
                                theme::background()
                            } else {
                                theme::surface()
                            })
                            .border_r_1()
                            .border_color(theme::border())
                            .on_press(move || {
                                workspace.update(|state| {
                                    activate_tab(state, id, tab_id);
                                    persist(state);
                                });
                            })
                            .child(text(title))
                            .child(
                                view()
                                    .text_color(theme::tertiary())
                                    .on_press(move || {
                                        workspace.update(|state| {
                                            close_tab(state, id, tab_id);
                                            persist(state);
                                        });
                                    })
                                    .child(text("×")),
                            )
                    })
                    .collect::<Vec<_>>();

                let header = view()
                    .h(px(32.0))
                    .flex_shrink_0()
                    .flex_row()
                    .items_center()
                    .bg(theme::surface())
                    .border_b_1()
                    .border_color(theme::border())
                    .children(tab_views)
                    .child(
                        view()
                            .h_full()
                            .w(px(32.0))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(14.0))
                            .text_color(theme::secondary())
                            .border_r_1()
                            .border_color(theme::border())
                            .on_press(move || {
                                workspace.update(|state| {
                                    add_tab(state, id);
                                    persist(state);
                                });
                            })
                            .child(text("+")),
                    )
                    .child(view().flex_1())
                    .child(
                        view()
                            .h_full()
                            .w(px(32.0))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(theme::tertiary())
                            .border_l_1()
                            .border_color(theme::border())
                            .label("Move pane")
                            .test_id(&format!("pane-move-{id}"))
                            .cursor(Cursor::Grab)
                            .map(|handle| drag_handle(handle, workspace, drag_state, id, false))
                            .child(text("⠿")),
                    )
                    .child(
                        view()
                            .h_full()
                            .w(px(32.0))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(theme::tertiary())
                            .border_l_1()
                            .border_color(theme::border())
                            .on_press(move || {
                                workspace.update(|state| {
                                    remove_pane(state, id);
                                    persist(state);
                                });
                            })
                            .child(text("×")),
                    );

                view()
                    .absolute()
                    .left(px(rect.x as f32 * step))
                    .top(px(GAP + rect.y as f32 * ROW_STEP))
                    .w(px((rect.w as f32 * step - GAP).max(1.0)))
                    .h(px((rect.h as f32 * ROW_STEP - GAP).max(1.0)))
                    .flex_col()
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::surface())
                    .child(header)
                    .child(view().flex_1().min_h(px(0.0)).bg(theme::background()))
                    .child(
                        view()
                            .absolute()
                            .right(px(0.0))
                            .bottom(px(0.0))
                            .size(px(18.0))
                            .flex()
                            .items_end()
                            .justify_end()
                            .p(px(3.0))
                            .cursor(Cursor::NwseResize)
                            .label("Resize pane")
                            .test_id(&format!("pane-resize-{id}"))
                            .map(|handle| drag_handle(handle, workspace, drag_state, id, true))
                            .child(
                                view()
                                    .size(px(7.0))
                                    .border_r_1()
                                    .border_b_1()
                                    .border_color(theme::tertiary()),
                            ),
                    )
            })
            .collect::<Vec<_>>();

        scroll()
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .scrollbar(true)
            .child(
                view()
                    .relative()
                    .w_full()
                    .h(px(height.max(1.0)))
                    .children(rendered),
            )
    })
}
