//! The shared workspace/grid view and its persisted page operations.
//!
//! Grid geometry remains in `layout/grid.rs`; this module adapts that model to
//! NSH elements and signals. The same component tree is used by desktop and
//! WebAssembly runners.

mod grid;
pub(crate) use grid::Grid;

use crate::{
    storage::{self, StoredPage, StoredWorkspace},
    theme,
};
use grid::{GAP, ROW_STEP, breakpoint, column_step, columns};
use nsh_core::prelude::*;

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

fn persist(workspace: &StoredWorkspace) {
    if let Err(error) = storage::save(workspace) {
        log::warn!("Lattice: unable to save workspace: {error}");
    }
}

/// Render the active responsive layout.
pub(crate) fn workspace(workspace: Signal<StoredWorkspace>) -> impl IntoElement {
    dynamic(move || {
        let saved = workspace.get();
        let width = (window().width.0 - 16.0).max(1.0);
        let bp = breakpoint(width);
        let cols = columns(bp);
        let step = column_step(width, cols);
        let panes = active_grid(&saved).layouts[bp].clone();
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
