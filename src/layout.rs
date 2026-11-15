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
//! A widget's view also gets its tab ([`TabHandle`], which the widgets'
//! host makes its kind's `TabContext` of): the settings saved in the tab
//! record, a way to save new ones, and the settings panel its pane's gear
//! opens for the active tab. A widget that can be linked ([`crate::link`])
//! also gets a link button in its pane's header, beside the gear, which
//! links it to a color; [`link_market`] shows a market on the widgets
//! linked to one. Which widget a tab shows is the widgets'
//! business ([`widgets::view`]); layout names none. A pane's `+` opens the app's widget picker
//! ([`Picker`]) for that pane, which adds a tab of the widget picked. A tab
//! dragged goes to another place among its pane's tabs, or into another pane
//! of its page (`layout/tab_drag.rs`), as the same widget: its content is
//! made again there, attached to the session it had.

mod grid;
mod tab_drag;
pub(crate) mod templates;
#[cfg(test)]
mod tests;
pub(crate) use grid::Grid;

use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

use crate::{
    data::{DataServices, Sessions, WidgetId, ferris::MarketSelection},
    link::LinkColor,
    storage::{self, StoredPage, StoredWorkspace},
    theme, ui,
    widgets::{
        self, SettingsPanel, TabHandle, WidgetKind, WidgetSettings,
        picker::{Picker, Target},
    },
};
use grid::{GAP, Pane, ROW_STEP, Tab, TabWidget, breakpoint, column_step, columns};
use nsh_core::{element::ImageSrc, prelude::*};
use tab_drag::{PaneDrag, TabDrags};

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

/// A wheel line over a pane's tabs, in points: a notch (three lines) moves
/// them by about a tab.
const TAB_WHEEL_LINE: f32 = 30.0;

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

/// Every widget's name (its kind's), on every page: what the connection
/// panel calls whoever asked for data. A widget of a kind this build does
/// not know has none.
pub(crate) fn widget_titles(workspace: &StoredWorkspace) -> HashMap<WidgetId, &'static str> {
    workspace
        .pages
        .iter()
        .flat_map(|page| page.grid.layouts.iter().flatten())
        .flat_map(|pane| pane.tabs.iter())
        .filter_map(|tab| Some((tab.widget, tab.kind()?.title())))
        .collect()
}

/// Saves `settings` in `widget`'s tab record, whichever page it is on;
/// false when no tab is that widget.
pub(crate) fn set_tab_settings(
    workspace: &mut StoredWorkspace,
    widget: WidgetId,
    settings: &WidgetSettings,
) -> bool {
    workspace
        .pages
        .iter_mut()
        .any(|page| page.grid.set_settings(widget, settings))
}

/// Links `widget` to `color` (or to none), whichever page it is on; false
/// when no tab is that widget, or it cannot be linked.
pub(crate) fn set_link(
    workspace: &mut StoredWorkspace,
    widget: WidgetId,
    color: Option<LinkColor>,
) -> bool {
    workspace
        .pages
        .iter_mut()
        .any(|page| page.grid.set_link(widget, color))
}

/// Shows `market` on every widget linked to `color`, on every page: what
/// a pick in the Everything Bar's search does while the bar has that color
/// ([`crate::link`]). The market goes into each widget's saved settings.
/// How many widgets that is.
pub(crate) fn link_market(
    workspace: &mut StoredWorkspace,
    color: LinkColor,
    market: &MarketSelection,
) -> usize {
    let mut sent = HashSet::new();
    for page in &mut workspace.pages {
        page.grid.link_market(color, market, &mut sent);
    }
    sent.len()
}

/// How many widgets are linked to each color, on every page, in
/// [`LinkColor::ALL`]'s order.
pub(crate) fn link_counts(workspace: &StoredWorkspace) -> [usize; 5] {
    let mut links = HashMap::new();
    for page in &workspace.pages {
        page.grid.links(&mut links);
    }
    LinkColor::ALL.map(|color| links.values().filter(|link| **link == Some(color)).count())
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

/// Moves a tab of the active page's pane `from` to a gap among pane `to`'s
/// tabs ([`Grid::move_tab`]); false when that changes nothing.
pub(crate) fn move_tab(
    workspace: &mut StoredWorkspace,
    from: usize,
    tab_id: usize,
    to: usize,
    gap: usize,
) -> bool {
    active_grid_mut(workspace).move_tab(from, tab_id, to, gap)
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
    // Tabs dragged from pane to pane, within the grid's scroll view and the
    // box the panes are placed in, which a tab held at an edge scrolls.
    let (viewport, content) = (element_ref(), element_ref());
    let drags = TabDrags::new(workspace, viewport, content);

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
        .element_ref(&viewport)
        .flex_1()
        .min_h(px(0.0))
        .w_full()
        .scrollbar(true)
        .child(
            view()
                .element_ref(&content)
                .relative()
                .w_full()
                .h(move || px(height.get().max(1.0)))
                .child(each(
                    panes,
                    |(page, pane)| (*page, pane.id),
                    move |pane, _| {
                        let sessions = sessions.clone();
                        pane_view(pane, step, workspace, drag_state, drags, picker, sessions)
                    },
                ))
                .child(drags.ghost()),
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
    drags: TabDrags,
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
    // The active tab's widget and the color it is linked to, when it is a
    // widget that can be linked: the header then has its link button.
    let link = computed(move || {
        let active = active_tab.get();
        tabs.with(|tabs| {
            tabs.iter()
                .find(|tab| tab.id == active)
                .and_then(|tab| match &tab.settings {
                    TabWidget::Known(settings) => settings.link().map(|color| (tab.widget, color)),
                    TabWidget::Unknown { .. } => None,
                })
        })
    });
    let linkable = computed(move || link.with(Option::is_some));
    let link_button = dynamic(move || {
        linkable.get().then(|| {
            ui::link_button(
                "Widget link color",
                move || link.get().and_then(|(_, color)| color),
                true,
                |_| String::new(),
                move |color| {
                    let Some((widget, _)) = link.get_untracked() else {
                        return;
                    };
                    workspace.update(|state| {
                        if set_link(state, widget, color) {
                            persist(state);
                        }
                    });
                },
                ui::Side::Right,
                8.0,
            )
            .h_full()
            .w(px(32.0))
            .border_l_1()
            .border_color(theme::BORDER)
            .test_id(&format!("pane-link-{id}"))
        })
    });

    // The `+` after the tabs, made with them.
    let add = move || {
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
            .child(text("+"))
    };

    // The tabs' scroll view, how wide it and the tabs are, and how far it
    // is scrolled: where a wheel left it, or a drag since (a touch pans it
    // natively, and reports where).
    let strip = element_ref();
    let strip_width = signal(0.0_f32);
    let tabs_width = signal(0.0_f32);
    let offset = signal(0.0_f32);
    let dragged = signal(false);
    // How far the tabs reach past their room.
    let hidden = computed(move || (tabs_width.get() - strip_width.get()).max(0.0));
    let overflows = computed(move || hidden.get() > 0.5);
    // Moves the tabs sideways by so much, within what they hide; whether
    // they moved.
    let scroll_by = move |by: f32| {
        let max = hidden.get_untracked();
        let from = offset.get_untracked().min(max);
        let to = (from + by).clamp(0.0, max);
        // Its own reports come back late: the offset is this one's.
        dragged.set(false);
        offset.set(to);
        strip.scroll_to(point(px(to), px(0.0)), false);
        to != from
    };
    // The pane takes tabs dragged to it ([`tab_drag`]), anywhere on its
    // box, and lets go of its own.
    let frame = element_ref();
    let drag = drags.pane(page, id, frame, strip, scroll_by);
    // A wheel over the tabs moves them sideways whichever way it turns: an
    // ordinary mouse wheel has no sideways turn, and left to the platform
    // it scrolls the page, the tabs being a sideways scroll inside it.
    let wheel = move |event: &WheelEvent| {
        if event.modifiers.ctrl {
            return;
        }
        let delta = event.delta.points(px(TAB_WHEEL_LINE));
        let by = if delta.x.0.abs() > delta.y.0.abs() {
            delta.x.0
        } else {
            delta.y.0
        };
        if by.is_finite() {
            scroll_by(by);
        }
    };
    // A tab added is the last and the active one: the tabs scroll to their
    // end, so it shows however narrow the pane.
    let tab_count = computed(move || tabs.with(Vec::len));
    let shown_tabs = std::cell::Cell::new(tab_count.get_untracked());
    effect(move || {
        let count = tab_count.get();
        if count > shown_tabs.replace(count) {
            // `spawn` here adds a pane.
            nsh_core::prelude::spawn(async move {
                // Once the new tab is laid out.
                next_frame().await;
                strip.scroll_to_end(false);
                dragged.set(false);
                offset.set(f32::MAX);
            });
        }
    });

    let header = view()
        .h(px(32.0))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .bg(theme::SURFACE)
        .border_b_1()
        .border_color(theme::BORDER)
        .child(
            // The room left of the controls: the tabs and the `+` after
            // them, which scroll sideways once the pane is narrower than
            // they are, so the controls never leave it.
            scroll()
                .horizontal()
                .scrollbar(false)
                .element_ref(&strip)
                .test_id(&format!("pane-tabs-{id}"))
                .h_full()
                .flex_1()
                .min_w(px(0.0))
                .on_layout(move |bounds: &Bounds<Points>| strip_width.set(bounds.size.width.0))
                .on_scroll_drag_start(move |_: &Point<Points>| dragged.set(true))
                .on_scroll(move |event: &ScrollEvent| {
                    if dragged.get_untracked() {
                        offset.set(event.offset.x.0);
                    }
                })
                .child(
                    view()
                        .h_full()
                        .flex_none()
                        .flex_row()
                        .on_layout(move |bounds: &Bounds<Points>| {
                            tabs_width.set(bounds.size.width.0)
                        })
                        // Made again when the tabs start or stop reaching
                        // past their room: they take the wheel only while
                        // they do (a listener inside a scroll view keeps
                        // the wheel from everything around it), so over
                        // tabs that all show it still scrolls the page.
                        .child(dynamic(move || {
                            let overflows = overflows.get();
                            view()
                                .h_full()
                                .flex_none()
                                .flex_row()
                                .items_center()
                                .test_id(&format!("pane-tab-row-{id}"))
                                .when(overflows, |row| row.on_wheel(wheel))
                                .child(dynamic(move || {
                                    let (tabs, active) = (tabs.get(), active_tab.get());
                                    tab_strip(id, &tabs, active, overflows, workspace, drag)
                                }))
                                .child(add())
                        })),
                ),
        )
        .child(link_button)
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
                .child(
                    svg(ImageSrc::svg_markup(include_str!("../assets/icons/settings.svg")))
                        .size(px(12.0)),
                ),
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
        .relative()
        .flex_1()
        .min_h(px(0.0))
        .flex_col()
        .bg(theme::BACKGROUND)
        .child(each(
            tabs,
            |tab: &Tab| tab.widget,
            move |tab, _| {
                let panel = SettingsPanel {
                    open: settings_open,
                    anchor: gear,
                };
                tab_content(tab, active_tab, &sessions, workspace, panel)
            },
        ))
        .child(tab_drag::drop_hint(move || {
            drag.targeted() && tab_count.get() == 0
        }));

    view()
        .absolute()
        .left(move || px(rect.get().x as f32 * step.get()))
        .top(move || px(GAP + rect.get().y as f32 * ROW_STEP))
        .w(move || px((rect.get().w as f32 * step.get() - GAP).max(1.0)))
        .h(move || px((rect.get().h as f32 * ROW_STEP - GAP).max(1.0)))
        .flex_col()
        .border_1()
        // Outlined while the tab in hand would go to it.
        .border_color(move || {
            if drag.targeted() {
                theme::ACCENT.get()
            } else {
                theme::BORDER.get()
            }
        })
        .bg(theme::SURFACE)
        .element_ref(&frame)
        .test_id(&format!("pane-{id}"))
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

/// A pane's tabs: buttons, cheap to make again, each of them one a drag
/// picks up ([`PaneDrag::tab`]). `overflows`: they reach past their room.
fn tab_strip(
    id: usize,
    tabs: &[Tab],
    active_tab: usize,
    overflows: bool,
    workspace: Signal<StoredWorkspace>,
    drag: PaneDrag,
) -> Vec<View> {
    drag.remake();
    tabs.iter()
        .enumerate()
        .map(|(index, tab)| {
            let tab_id = tab.id;
            let selected = tab_id == active_tab;
            let button = view()
                .h_full()
                // Whole however narrow the pane: the strip scrolls.
                .flex_none()
                .flex()
                .items_center()
                .gap(px(6.0))
                .px(px(12.0))
                .text_size(px(11.0))
                .test_id(&format!("pane-tab-{id}-{tab_id}"))
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
                );
            drag.tab(button, tab, index + 1 == tabs.len(), overflows, move || {
                workspace.update(|state| {
                    activate_tab(state, id, tab_id);
                    persist(state);
                });
            })
        })
        .collect()
}

/// One tab's content: mounted while its pane is, hidden while another tab
/// of the pane is active, and attached to its widget's session meanwhile.
fn tab_content(
    tab: ReadSignal<Tab>,
    active_tab: Memo<usize>,
    sessions: &Sessions,
    workspace: Signal<StoredWorkspace>,
    panel: SettingsPanel,
) -> View {
    // The row is keyed by widget, whose tab id and kind never change.
    let (tab_id, widget, kind) = tab.with_untracked(|tab| (tab.id, tab.widget, tab.kind()));
    let shown = computed(move || active_tab.get() == tab_id);
    let attached = sessions.attach(widget, shown);
    let body = match kind {
        Some(kind) => widgets::view(&attached, tab_handle(tab, kind, shown, workspace, panel)),
        None => unavailable(tab),
    };
    attached.release_on_cleanup();
    view()
        .flex_1()
        .min_h(px(0.0))
        .flex_col()
        .test_id(&format!("widget-{widget}"))
        .style_when(move || !shown.get(), |s| s.hidden())
        .child(body)
}

/// What `tab`'s widget gets from it: its saved settings, a way to save
/// new ones (every breakpoint's copy of the tab, then the workspace), and
/// its pane's settings panel.
fn tab_handle(
    tab: ReadSignal<Tab>,
    kind: WidgetKind,
    shown: Memo<bool>,
    workspace: Signal<StoredWorkspace>,
    panel: SettingsPanel,
) -> TabHandle {
    let widget = tab.with_untracked(|tab| tab.widget);
    TabHandle {
        kind,
        read: Rc::new(move |read: &mut dyn FnMut(&WidgetSettings)| {
            tab.with(|tab| {
                if let TabWidget::Known(settings) = &tab.settings {
                    read(settings);
                }
            });
        }),
        shown,
        save: Rc::new(move |next: &WidgetSettings| {
            // Nothing when unchanged, or when the tab is gone.
            let changed = workspace.with_untracked(|state| {
                widget_ids(state).contains(&widget)
                    && tab.with_untracked(
                        |tab| !matches!(&tab.settings, TabWidget::Known(saved) if saved == next),
                    )
            });
            if changed {
                workspace.update(|state| {
                    if set_tab_settings(state, widget, next) {
                        persist(state);
                    }
                });
            }
        }),
        panel,
    }
}

/// A tab whose widget this build does not know (a newer build saved it):
/// it says so, and its record is kept as saved, so that build still finds
/// its widget there.
fn unavailable(tab: ReadSignal<Tab>) -> View {
    let kind = tab.with_untracked(|tab| match &tab.settings {
        TabWidget::Unknown { kind, .. } => kind.clone(),
        TabWidget::Known(_) => String::new(),
    });
    view()
        .flex_1()
        .min_h(px(0.0))
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .px(px(20.0))
        .py(px(40.0))
        .text_center()
        .text_size(px(12.0))
        .text_color(theme::TEXT_MUTED)
        .role(AccessibilityRole::Text)
        .test_id("widget-unavailable")
        .child(text(
            "This widget is not available in this version of Lattice.",
        ))
        .child(
            text(format!("Widget: {kind}"))
                .text_size(px(11.0))
                .text_color(theme::TEXT_FAINT),
        )
}
