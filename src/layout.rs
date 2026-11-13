//! The shared workspace/grid view and its persisted page operations.
//!
//! Grid geometry remains in `layout/grid.rs`; this module adapts that model to
//! NSH elements and signals. The same component tree is used by desktop and
//! WebAssembly runners.
//!
//! The grid is keyed, so a change rebuilds only what it changed: panes are
//! rows of an `each` by page and pane id, whose geometry follows the layout
//! and window width as bindings; a pane's tab strip is made again when its
//! tabs or active tab change; and its tabs' contents are rows by widget id,
//! every tab of the pane mounted, the inactive ones hidden. Tab switches,
//! drags (every pointer sample), resizes and breakpoint changes therefore
//! keep every widget's view. Each content view attaches to its widget's
//! data session ([`crate::data::Sessions`]), visible while its tab is the
//! active one; other pages are not mounted, so their widgets' sessions keep
//! their data with no view attached.
//!
//! A widget's view also gets its tab ([`TabContext`]): the settings saved in
//! the tab record, a way to save new ones, and the settings panel its pane's
//! gear opens for the active tab. A pane's `+` opens the app's widget picker
//! ([`Picker`]) for that pane, which adds a tab of the widget picked.

mod grid;
#[cfg(test)]
mod tests;
pub(crate) use grid::Grid;

use std::{collections::HashSet, rc::Rc};

use crate::{
    data::{DataServices, Sessions, WidgetId, WidgetView},
    storage::{self, StoredPage, StoredWorkspace},
    theme,
    widgets::{
        self, WidgetKind,
        market_screener::ScreenerSettings,
        picker::{Picker, Target},
    },
};
use grid::{GAP, Pane, ROW_STEP, Tab, breakpoint, column_step, columns};
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
        page.grid.retitle_blank_tabs();
        page.grid.unify_widgets();
    }
}

/// Every widget in the workspace, on every page: what keeps a session.
pub(crate) fn widget_ids(workspace: &StoredWorkspace) -> HashSet<WidgetId> {
    workspace
        .pages
        .iter()
        .flat_map(|page| page.grid.layouts.iter().flatten())
        .flat_map(|pane| pane.tabs.iter().map(|tab| tab.widget))
        .collect()
}

/// Saves `settings` in `widget`'s tab record, whichever page it is on;
/// false when no tab is that widget.
pub(crate) fn set_tab_settings(
    workspace: &mut StoredWorkspace,
    widget: WidgetId,
    settings: &ScreenerSettings,
) -> bool {
    workspace
        .pages
        .iter_mut()
        .any(|page| page.grid.set_settings(widget, settings))
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

/// The Layout Manager's "Clear": delete a page and its widgets, selecting the
/// page before it when it was active. The last remaining page is emptied
/// instead, since the workspace always keeps one page.
pub(crate) fn clear_page(workspace: &mut StoredWorkspace, page_id: usize) -> bool {
    let Some(index) = workspace.pages.iter().position(|page| page.id == page_id) else {
        return false;
    };
    if workspace.pages.len() == 1 {
        workspace.pages[0].grid = Grid::default();
        return true;
    }
    workspace.pages.remove(index);
    if workspace.active_page_id == page_id {
        workspace.active_page_id = workspace.pages[index.saturating_sub(1)].id;
    }
    true
}

pub(crate) fn export_code(workspace: &StoredWorkspace) -> Result<String, String> {
    let bytes = serde_json::to_vec(workspace).map_err(|error| error.to_string())?;
    let mut code = String::from("LATTICE1.");
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut code, "{byte:02x}").map_err(|error| error.to_string())?;
    }
    Ok(code)
}

pub(crate) fn import_code(code: &str) -> Result<StoredWorkspace, String> {
    let payload = code
        .trim()
        .strip_prefix("LATTICE1.")
        .ok_or_else(|| "Code must start with LATTICE1.".to_owned())?;
    if payload.is_empty() || payload.len() % 2 != 0 {
        return Err("Layout code is incomplete.".to_owned());
    }
    let bytes = payload
        .as_bytes()
        .chunks(2)
        .map(|pair| {
            let hex = std::str::from_utf8(pair).map_err(|error| error.to_string())?;
            u8::from_str_radix(hex, 16).map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut workspace: StoredWorkspace =
        serde_json::from_slice(&bytes).map_err(|_| "Layout code is invalid.".to_owned())?;
    if workspace.version != 1 {
        return Err("This layout code version is not supported.".to_owned());
    }
    normalize(&mut workspace);
    Ok(workspace)
}

/// Adds a pane holding one `kind` widget to the active page.
pub(crate) fn spawn(workspace: &mut StoredWorkspace, kind: WidgetKind) {
    active_grid_mut(workspace).spawn(kind);
}

/// Whether the active page has the pane `pane_id`.
pub(crate) fn has_pane(workspace: &StoredWorkspace, pane_id: usize) -> bool {
    active_grid(workspace)
        .layouts
        .iter()
        .flatten()
        .any(|pane| pane.id == pane_id)
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

pub(crate) fn add_tab(workspace: &mut StoredWorkspace, pane_id: usize, kind: WidgetKind) {
    active_grid_mut(workspace).add_tab(pane_id, kind);
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

pub(crate) fn persist(workspace: &StoredWorkspace) {
    if let Err(error) = storage::save(workspace) {
        log::warn!("Lattice: unable to save workspace: {error}");
    }
}

/// Render the active responsive layout.
pub(crate) fn workspace(workspace: Signal<StoredWorkspace>, picker: Picker) -> impl IntoElement {
    // Standalone (some tests), the grid keeps its own registry.
    let sessions = DataServices::try_get()
        .map(|services| services.sessions)
        .unwrap_or_default();
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

    let width = computed(workspace_width);
    let step = computed(move || {
        let width = width.get();
        column_step(width, columns(breakpoint(width)))
    });
    // The panes on screen, with their page: a pane id is unique only within
    // its page, so the page is part of the row key.
    let panes = computed(move || {
        let saved = workspace.get();
        let width = width.get();
        let page = saved.active_page_id;
        drag_state
            .get()
            .filter(|drag| drag.valid(&saved, width))
            .map(|drag| drag.preview)
            .unwrap_or_else(|| active_grid(&saved).layouts[breakpoint(width)].clone())
            .into_iter()
            .map(|pane| (page, pane))
            .collect::<Vec<_>>()
    });
    let height = computed(move || {
        panes.with(|panes| {
            panes
                .iter()
                .map(|(_, pane)| pane.rect.y + pane.rect.h)
                .max()
                .unwrap_or(0) as f32
                * ROW_STEP
                + GAP
        })
    });

    scroll()
        .flex_1()
        .min_h(px(0.0))
        .w_full()
        .scrollbar(true)
        .child(
            view()
                .relative()
                .w_full()
                .h(move || px(height.get().max(1.0)))
                .child(each(
                    panes,
                    |(page, pane)| (*page, pane.id),
                    move |pane, _| {
                        pane_view(pane, step, workspace, drag_state, picker, sessions.clone())
                    },
                )),
        )
}

/// One pane: its geometry follows the layout as bindings, its tab strip is
/// made again only when its tabs or active tab change, and each tab's
/// content is a row of its own.
fn pane_view(
    pane: ReadSignal<(usize, Pane)>,
    step: Memo<f32>,
    workspace: Signal<StoredWorkspace>,
    drag_state: Signal<Option<DragState>>,
    picker: Picker,
    sessions: Sessions,
) -> impl IntoElement {
    // The row's key: the same for as long as the row lives.
    let (page, id) = pane.with_untracked(|(page, pane)| (*page, pane.id));
    // What the `+` opens the picker for.
    let add_here = Target::Tab { page, pane: id };
    let rect = computed(move || pane.with(|(_, pane)| pane.rect));
    let tabs = computed(move || pane.with(|(_, pane)| pane.tabs.clone()));
    let active_tab = computed(move || pane.with(|(_, pane)| pane.active_tab));
    // The active tab's settings panel, which the gear toggles; switching
    // tabs closes it, as the old app did.
    let settings_open = signal(false);
    let gear = element_ref();
    effect(move || {
        active_tab.get();
        settings_open.set(false);
    });

    let header = view()
        .h(px(32.0))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .bg(theme::SURFACE)
        .border_b_1()
        .border_color(theme::BORDER)
        .child(dynamic(move || {
            tab_strip(id, &tabs.get(), active_tab.get(), workspace)
        }))
        .child(
            view()
                .h_full()
                .w(px(32.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(14.0))
                .text_color(move || {
                    if picker.is_open_for(add_here) {
                        theme::ACCENT.get()
                    } else {
                        theme::TEXT_MUTED.get()
                    }
                })
                .border_r_1()
                .border_color(theme::BORDER)
                .role(AccessibilityRole::Button)
                .label("Add widget tab")
                .test_id(&format!("pane-add-{id}"))
                .accessibility_state(move || AccessibilityState {
                    expanded: Some(picker.is_open_for(add_here)),
                    has_popup: true,
                    ..Default::default()
                })
                .hover(|s| s.bg(theme::BACKGROUND))
                .active(|s| s.bg(theme::BORDER))
                .on_press(move || picker.toggle(add_here))
                .child(text("+")),
        )
        .child(view().flex_1())
        .child(
            view()
                .element_ref(&gear)
                .h_full()
                .w(px(32.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .text_color(move || {
                    if settings_open.get() {
                        theme::ACCENT.get()
                    } else {
                        theme::TEXT_FAINT.get()
                    }
                })
                .border_l_1()
                .border_color(theme::BORDER)
                .role(AccessibilityRole::Button)
                .label("Widget settings")
                .test_id(&format!("pane-settings-{id}"))
                .accessibility_state(move || AccessibilityState {
                    expanded: Some(settings_open.get()),
                    has_popup: true,
                    ..Default::default()
                })
                .hover(|s| s.bg(theme::BACKGROUND))
                .active(|s| s.bg(theme::BORDER))
                .on_press(move || settings_open.update(|open| *open = !*open))
                .child(text("⚙")),
        )
        .child(
            view()
                .h_full()
                .w(px(32.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme::TEXT_FAINT)
                .border_l_1()
                .border_color(theme::BORDER)
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
                .text_color(theme::TEXT_FAINT)
                .border_l_1()
                .border_color(theme::BORDER)
                .on_press(move || {
                    workspace.update(|state| {
                        remove_pane(state, id);
                        persist(state);
                    });
                })
                .child(text("×")),
        );

    let content = view()
        .flex_1()
        .min_h(px(0.0))
        .flex_col()
        .bg(theme::BACKGROUND)
        .child(each(
            tabs,
            |tab: &Tab| tab.widget,
            move |tab, _| {
                let panel = Panel {
                    open: settings_open,
                    anchor: gear,
                };
                tab_content(tab, active_tab, &sessions, workspace, panel)
            },
        ));

    view()
        .absolute()
        .left(move || px(rect.get().x as f32 * step.get()))
        .top(move || px(GAP + rect.get().y as f32 * ROW_STEP))
        .w(move || px((rect.get().w as f32 * step.get() - GAP).max(1.0)))
        .h(move || px((rect.get().h as f32 * ROW_STEP - GAP).max(1.0)))
        .flex_col()
        .border_1()
        .border_color(theme::BORDER)
        .bg(theme::SURFACE)
        .child(header)
        .child(content)
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
                        .border_color(theme::TEXT_FAINT),
                ),
        )
}

/// A pane's tabs: stateless buttons, cheap to make again.
fn tab_strip(
    id: usize,
    tabs: &[Tab],
    active_tab: usize,
    workspace: Signal<StoredWorkspace>,
) -> Vec<View> {
    tabs.iter()
        .map(|tab| {
            let tab_id = tab.id;
            let selected = tab_id == active_tab;
            view()
                .h_full()
                .flex()
                .items_center()
                .gap(px(6.0))
                .px(px(12.0))
                .text_size(px(11.0))
                .text_color(if selected {
                    theme::TEXT
                } else {
                    theme::TEXT_MUTED
                })
                .bg(if selected {
                    theme::BACKGROUND
                } else {
                    theme::SURFACE
                })
                .border_r_1()
                .border_color(theme::BORDER)
                .on_press(move || {
                    workspace.update(|state| {
                        activate_tab(state, id, tab_id);
                        persist(state);
                    });
                })
                .child(text(tab.title.clone()))
                .child(
                    view()
                        .text_color(theme::TEXT_FAINT)
                        .on_press(move || {
                            workspace.update(|state| {
                                close_tab(state, id, tab_id);
                                persist(state);
                            });
                        })
                        .child(text("×")),
                )
        })
        .collect()
}

/// A pane's settings panel: whether it is open, and the gear it opens
/// below.
#[derive(Clone, Copy)]
struct Panel {
    open: Signal<bool>,
    anchor: ElementRef,
}

/// What a widget's view gets from its tab, besides its session: the
/// settings saved in the tab record, and its pane's settings panel.
#[derive(Clone)]
pub(crate) struct TabContext {
    /// The tab's saved settings.
    pub settings: Memo<ScreenerSettings>,
    /// Whether the tab is its pane's active one (its view shows).
    pub shown: Memo<bool>,
    save: Rc<dyn Fn(&ScreenerSettings)>,
    panel: Panel,
}

impl TabContext {
    /// Saves `settings` in the tab record (every breakpoint's copy of the
    /// tab) and persists the workspace.
    pub fn save(&self, settings: &ScreenerSettings) {
        (self.save)(settings);
    }

    /// Whether the widget's settings panel is open: the pane's gear opened
    /// it while this tab is the active one (tracked).
    pub fn settings_open(&self) -> bool {
        self.shown.get() && self.panel.open.get()
    }

    pub fn close_settings(&self) {
        self.panel.open.set(false);
    }

    /// The gear the settings panel opens below.
    pub fn settings_anchor(&self) -> &ElementRef {
        &self.panel.anchor
    }
}

/// One tab's content: mounted while its pane is, hidden while another tab
/// of the pane is active, and attached to its widget's session meanwhile.
fn tab_content(
    tab: ReadSignal<Tab>,
    active_tab: Memo<usize>,
    sessions: &Sessions,
    workspace: Signal<StoredWorkspace>,
    panel: Panel,
) -> View {
    // The row is keyed by widget, whose tab id never changes.
    let (tab_id, widget) = tab.with_untracked(|tab| (tab.id, tab.widget));
    let shown = computed(move || active_tab.get() == tab_id);
    let context = TabContext {
        settings: computed(move || tab.with(|tab| tab.settings.clone())),
        shown,
        save: Rc::new(move |settings: &ScreenerSettings| {
            // Nothing when unchanged, or when the tab is gone.
            let changed = workspace.with_untracked(|state| {
                widget_ids(state).contains(&widget)
                    && tab.with_untracked(|tab| tab.settings != *settings)
            });
            if changed {
                workspace.update(|state| {
                    if set_tab_settings(state, widget, settings) {
                        persist(state);
                    }
                });
            }
        }),
        panel,
    };
    let attached = sessions.attach(widget, shown);
    let body = widget_body(&attached, &context);
    attached.release_on_cleanup();
    view()
        .flex_1()
        .min_h(px(0.0))
        .flex_col()
        .test_id(&format!("widget-{widget}"))
        .style_when(move || !shown.get(), |s| s.hidden())
        .child(body)
}

/// What a widget shows, from its session (`attached.state(..)` holds its
/// data) and its tab. The Market Screener is the only widget so far, so
/// every tab is one.
fn widget_body(attached: &WidgetView, tab: &TabContext) -> View {
    widgets::market_screener::market_screener(attached, tab)
}
