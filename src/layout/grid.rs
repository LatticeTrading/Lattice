//! Integer grid geometry, independent of GPUI. Row height and gaps match lattice-pro.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{data::WidgetId, widgets::market_screener::ScreenerSettings};

pub const GAP: f32 = 8.;
pub const ROW: f32 = 8.;
pub const ROW_STEP: f32 = ROW + GAP;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tab {
    /// Unique within its page's grid only; the same in every breakpoint.
    pub id: usize,
    pub title: String,
    /// The widget this tab is, which keys its data session. Not saved: a
    /// tab that is loaded, imported or created is a new widget with a fresh
    /// id, whatever page and tab ids it reuses ([`Grid::unify_widgets`]
    /// makes a loaded tab's breakpoint copies share one).
    #[serde(skip, default = "WidgetId::fresh")]
    pub widget: WidgetId,
    /// The widget's settings (a Market Screener's: every tab is one), the
    /// same in every breakpoint's copy. Left out of the saved form while
    /// they are the defaults, so a tab nobody configured saves as it did
    /// before settings existed.
    #[serde(default, skip_serializing_if = "ScreenerSettings::is_default")]
    pub settings: ScreenerSettings,
}

/// A new tab's title: the Market Screener is the only widget so far.
pub const NEW_TAB_TITLE: &str = "Market Screener";
/// What tabs were called before widgets existed.
const BLANK_TAB_TITLE: &str = "Blank Widget";

impl Tab {
    fn new(id: usize) -> Self {
        Self {
            id,
            title: NEW_TAB_TITLE.into(),
            widget: WidgetId::fresh(),
            settings: ScreenerSettings::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Pane {
    pub id: usize,
    pub rect: Rect,
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
}

pub fn breakpoint(width: f32) -> usize {
    [480., 768., 996., 1200.].partition_point(|edge| width >= *edge)
}

pub fn columns(breakpoint: usize) -> u32 {
    [8, 16, 24, 40, 48][breakpoint]
}

pub fn column_step(width: f32, columns: u32) -> f32 {
    ((width + GAP) / columns as f32).max(1.)
}

pub fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

/// Compact in visual order. A resized pane keeps its anchor and pushes colliding
/// neighbours down; there are no overlapping committed panes or pixel drift.
pub fn compact(panes: &mut [Pane], pinned: Option<usize>) {
    let mut order: Vec<_> = (0..panes.len()).collect();
    order.sort_by_key(|&i| {
        (
            Some(panes[i].id) != pinned,
            panes[i].rect.y,
            panes[i].rect.x,
            panes[i].id,
        )
    });
    let mut placed: Vec<Rect> = Vec::new();
    for i in order {
        let mut rect = panes[i].rect;
        if Some(panes[i].id) != pinned {
            rect.y = 0;
        }
        while let Some(bottom) = placed
            .iter()
            .filter(|other| overlaps(rect, **other))
            .map(|other| other.y + other.h)
            .max()
        {
            rect.y = bottom;
        }
        panes[i].rect = rect;
        placed.push(rect);
    }
}

pub fn moved(rect: Rect, dx: f32, dy: f32, width: f32, cols: u32) -> Rect {
    let step = column_step(width, cols);
    Rect {
        x: (rect.x as f32 + dx / step)
            .round()
            .clamp(0., (cols - rect.w) as f32) as u32,
        y: (rect.y as f32 + dy / ROW_STEP).round().max(0.) as u32,
        ..rect
    }
}

pub fn resized(rect: Rect, dx: f32, dy: f32, width: f32, cols: u32) -> Rect {
    // Keep room for the future tab strip, even at the narrowest breakpoint.
    let min_w = ((160. + GAP) / column_step(width, cols)).ceil() as u32;
    Rect {
        w: (rect.w as f32 + dx / column_step(width, cols))
            .round()
            .clamp(min_w.min(cols - rect.x) as f32, (cols - rect.x) as f32) as u32,
        h: (rect.h as f32 + dy / ROW_STEP).round().clamp(6., 4096.) as u32,
        ..rect
    }
}

/// Apply one drag from a stable snapshot. Rebuilding from the snapshot keeps
/// pointer samples from compounding rounding or collision movement.
pub fn dragged(
    before: &[Pane],
    pane_id: usize,
    dx: f32,
    dy: f32,
    width: f32,
    breakpoint: usize,
    resize: bool,
) -> Vec<Pane> {
    let cols = columns(breakpoint);
    let mut layout = before.to_vec();
    let Some(pane) = layout.iter_mut().find(|pane| pane.id == pane_id) else {
        return layout;
    };
    let rect = if resize {
        resized(pane.rect, dx, dy, width, cols)
    } else {
        moved(pane.rect, dx, dy, width, cols)
    };
    // A click, sub-cell movement, or a return to the starting cell must
    // restore the exact snapshot, not compact otherwise untouched panes.
    if rect == pane.rect {
        return layout;
    }
    pane.rect = rect;
    compact(&mut layout, Some(pane_id));
    layout
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Grid {
    pub layouts: [Vec<Pane>; 5],
    next_id: usize,
    next_tab_id: usize,
}

impl Grid {
    /// Repair counters after loading a workspace written by an older build.
    /// Layout contents remain authoritative; IDs are only advanced, never
    /// regenerated, so saved tabs keep their identity.
    pub fn repair_counters(&mut self) {
        self.next_id = self
            .layouts
            .iter()
            .flat_map(|layout| layout.iter().map(|pane| pane.id))
            .max()
            .unwrap_or(0)
            .max(self.next_id);
        self.next_tab_id = self
            .layouts
            .iter()
            .flat_map(|layout| {
                layout
                    .iter()
                    .flat_map(|pane| pane.tabs.iter().map(|tab| tab.id))
            })
            .max()
            .unwrap_or(0)
            .max(self.next_tab_id);
    }

    /// Tabs saved before widgets existed were blank widgets; they are
    /// Market Screeners now, and say so.
    pub fn retitle_blank_tabs(&mut self) {
        for tab in self
            .layouts
            .iter_mut()
            .flatten()
            .flat_map(|pane| &mut pane.tabs)
        {
            if tab.title == BLANK_TAB_TITLE {
                NEW_TAB_TITLE.clone_into(&mut tab.title);
            }
        }
    }

    /// Gives each tab one widget id and one set of settings in every
    /// breakpoint's copy of it. A deserialized tab's copies each got a
    /// fresh id; the first copy's id and settings are kept for all of them
    /// (the settings differ only in a save edited by hand). A tab is known
    /// by its pane and tab ids, as the tab operations find it.
    pub fn unify_widgets(&mut self) {
        let mut first: HashMap<(usize, usize), (WidgetId, ScreenerSettings)> = HashMap::new();
        for pane in self.layouts.iter_mut().flatten() {
            let pane_id = pane.id;
            for tab in &mut pane.tabs {
                let (widget, settings) = first
                    .entry((pane_id, tab.id))
                    .or_insert_with(|| (tab.widget, tab.settings.clone()));
                tab.widget = *widget;
                if tab.settings != *settings {
                    tab.settings = settings.clone();
                }
            }
        }
    }

    /// Saves `settings` in every copy of `widget`'s tab; false when no tab
    /// of this grid is that widget.
    pub fn set_settings(&mut self, widget: WidgetId, settings: &ScreenerSettings) -> bool {
        let mut found = false;
        for tab in self
            .layouts
            .iter_mut()
            .flatten()
            .flat_map(|pane| &mut pane.tabs)
            .filter(|tab| tab.widget == widget)
        {
            settings.clone_into(&mut tab.settings);
            found = true;
        }
        found
    }

    pub fn spawn(&mut self) -> usize {
        self.next_id += 1;
        let id = self.next_id;
        self.next_tab_id += 1;
        let tab = Tab::new(self.next_tab_id);
        for (bp, layout) in self.layouts.iter_mut().enumerate() {
            let y = layout
                .iter()
                .map(|pane| pane.rect.y + pane.rect.h)
                .max()
                .unwrap_or(0);
            layout.push(Pane {
                id,
                rect: Rect {
                    x: 0,
                    y,
                    w: 24.min(columns(bp)),
                    h: 30,
                },
                tabs: vec![tab.clone()],
                active_tab: tab.id,
            });
        }
        id
    }

    pub fn remove_pane(&mut self, pane_id: usize) {
        for layout in &mut self.layouts {
            layout.retain(|pane| pane.id != pane_id);
        }
    }

    pub fn add_tab(&mut self, pane_id: usize) {
        self.next_tab_id += 1;
        let tab = Tab::new(self.next_tab_id);
        for layout in &mut self.layouts {
            if let Some(pane) = layout.iter_mut().find(|pane| pane.id == pane_id) {
                pane.tabs.push(tab.clone());
                pane.active_tab = tab.id;
            }
        }
    }

    pub fn activate_tab(&mut self, pane_id: usize, tab_id: usize) {
        for layout in &mut self.layouts {
            if let Some(pane) = layout.iter_mut().find(|pane| pane.id == pane_id)
                && pane.tabs.iter().any(|tab| tab.id == tab_id)
            {
                pane.active_tab = tab_id;
            }
        }
    }

    pub fn close_tab(&mut self, pane_id: usize, tab_id: usize) {
        for layout in &mut self.layouts {
            if let Some(pane) = layout.iter_mut().find(|pane| pane.id == pane_id) {
                let Some(index) = pane.tabs.iter().position(|tab| tab.id == tab_id) else {
                    continue;
                };
                pane.tabs.remove(index);
                if pane.active_tab == tab_id {
                    pane.active_tab = pane
                        .tabs
                        .get(index.min(pane.tabs.len().saturating_sub(1)))
                        .or_else(|| pane.tabs.first())
                        .map(|tab| tab.id)
                        .unwrap_or(0);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responsive_spawn_is_bounded_and_unique() {
        let mut grid = Grid::default();
        for id in 1..=100 {
            assert_eq!(grid.spawn(), id);
        }
        for (bp, layout) in grid.layouts.iter().enumerate() {
            for (index, pane) in layout.iter().enumerate() {
                assert!(pane.rect.x + pane.rect.w <= columns(bp));
                assert!(
                    layout[..index]
                        .iter()
                        .all(|other| !overlaps(pane.rect, other.rect))
                );
            }
        }
        for (width, expected) in [(479., 0), (480., 1), (768., 2), (996., 3), (1200., 4)] {
            assert_eq!(breakpoint(width), expected);
        }
    }

    #[test]
    fn move_snaps_and_stays_inside_columns() {
        let rect = Rect {
            x: 8,
            y: 4,
            w: 16,
            h: 20,
        };
        assert_eq!(moved(rect, 10000., -10000., 1264., 48).x, 32);
        assert_eq!(moved(rect, 10000., -10000., 1264., 48).y, 0);
        assert_eq!(moved(rect, 1., 1., 1264., 48), rect);
    }

    #[test]
    fn remove_pane_removes_it_from_every_breakpoint() {
        let mut grid = Grid::default();
        let pane_id = grid.spawn();
        grid.spawn();
        grid.remove_pane(pane_id);
        assert!(
            grid.layouts
                .iter()
                .all(|layout| layout.iter().all(|pane| pane.id != pane_id))
        );
        assert!(grid.layouts.iter().all(|layout| layout.len() == 1));
    }

    #[test]
    fn tabs_add_activate_and_close_without_removing_the_pane() {
        let mut grid = Grid::default();
        let pane_id = grid.spawn();
        let first_tab = grid.layouts[4][0].active_tab;
        grid.add_tab(pane_id);
        let second_tab = grid.layouts[4][0].active_tab;
        assert_ne!(first_tab, second_tab);
        assert_eq!(grid.layouts[4][0].tabs.len(), 2);
        grid.activate_tab(pane_id, first_tab);
        assert_eq!(grid.layouts[4][0].active_tab, first_tab);
        grid.close_tab(pane_id, first_tab);
        assert_eq!(grid.layouts[4][0].tabs.len(), 1);
        assert_eq!(grid.layouts[4][0].active_tab, second_tab);
        grid.close_tab(pane_id, second_tab);
        assert!(grid.layouts[4][0].tabs.is_empty());
        assert_eq!(grid.layouts[4][0].active_tab, 0);
    }

    #[test]
    fn drag_rebuilds_from_snapshot_and_compacts_neighbours() {
        let mut grid = Grid::default();
        let first = grid.spawn();
        grid.spawn();
        let before = grid.layouts[4].clone();
        let moved_once = dragged(&before, first, 1000.0, 32.0, 1264.0, 4, false);
        let moved_again = dragged(&before, first, 1000.0, 32.0, 1264.0, 4, false);
        assert_eq!(moved_once, moved_again);
        assert_eq!(moved_once[0].rect.x, 24);
        assert_eq!(moved_once[0].rect.y, 2);
        assert_eq!(moved_once[1].rect.y, 0);
    }

    #[test]
    fn resize_snaps_clamps_and_compacts_neighbours() {
        let mut grid = Grid::default();
        grid.spawn();
        grid.spawn();
        let layout = &mut grid.layouts[4];
        let original = layout[0].rect;
        layout[0].rect = resized(original, 10000., 32., 1264., 48);
        assert_eq!(layout[0].rect.w, 48);
        assert_eq!(layout[0].rect.h, 32);
        compact(layout, Some(1));
        assert_eq!(layout[1].rect.y, 32);
        layout[0].rect = resized(original, -10000., -10000., 1264., 48);
        assert!(layout[0].rect.w >= 6);
        assert_eq!(layout[0].rect.h, 6);
        compact(layout, Some(1));
        assert_eq!(layout[1].rect.y, 6);
        assert_eq!(resized(original, 1., 1., 1264., 48), original);
    }

    #[test]
    fn compaction_preserves_side_by_side_tiles() {
        let mut panes = [
            Pane {
                id: 1,
                rect: Rect {
                    x: 0,
                    y: 10,
                    w: 24,
                    h: 30,
                },
                tabs: vec![],
                active_tab: 0,
            },
            Pane {
                id: 2,
                rect: Rect {
                    x: 24,
                    y: 20,
                    w: 24,
                    h: 30,
                },
                tabs: vec![],
                active_tab: 0,
            },
        ];
        compact(&mut panes, None);
        assert_eq!(panes[0].rect.y, 0);
        assert_eq!(panes[1].rect.y, 0);
        assert!(!overlaps(panes[0].rect, panes[1].rect));
    }
}
