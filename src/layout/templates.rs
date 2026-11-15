//! The default layouts: arrangements of widgets to start a page from (the
//! old app's Community Layouts, `communityLayouts.ts`), which the Layout
//! Manager lists with a preview of each.
//!
//! A template places its widgets on a design of [`COLUMNS`] by [`ROWS`]
//! cells, the one its preview draws. A page made from it gets that design
//! at every breakpoint: scaled to the breakpoint's columns where there are
//! more than [`STACKED_COLUMNS`], and one widget under another, each the
//! full width, where there are not (a chart squeezed into a few columns
//! shows nothing worth reading). A design row is as many grid rows as it
//! takes for the six of them to fill the workspace the page opens in, so a
//! new page is a window of widgets with nothing below to scroll to.
//!
//! A widget is its kind and, where the template wants other than the
//! kind's defaults, the members its settings save (a chart's `timeframe`),
//! read as a saved tab's are.

use serde_json::Value;

use super::{
    active_grid, active_grid_mut, add_page,
    grid::{GAP, Grid, Pane, ROW_STEP, Rect, Tab, TabWidget, columns},
};
use crate::{
    data::WidgetId,
    storage::StoredWorkspace,
    widgets::{WidgetKind, WidgetSettings},
};

/// The design's columns.
pub(crate) const COLUMNS: u32 = 12;
/// The design's rows.
pub(crate) const ROWS: u32 = 6;
/// Breakpoints with no more columns than this stack the widgets.
const STACKED_COLUMNS: u32 = 16;
/// The fewest and the most grid rows a stacked widget is high, whatever its
/// height on the design.
const STACKED_ROWS: (u32, u32) = (20, 32);
/// The fewest and the most grid rows a design row is: a widget two design
/// rows high stays usable in a short window (the page then scrolls), and a
/// very tall window does not make a page of giants.
const UNIT: (u32, u32) = (6, 24);

/// One widget of a template: what it is and where it is on the design.
pub(crate) struct Panel {
    /// What the preview calls it.
    pub label: &'static str,
    pub kind: WidgetKind,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// The members its settings save that are not its kind's defaults.
    members: &'static [(&'static str, &'static str)],
}

impl Panel {
    /// A `kind` widget with its default settings, at `[x, y, w, h]` on the
    /// design.
    const fn new(label: &'static str, kind: WidgetKind, [x, y, w, h]: [u32; 4]) -> Self {
        Self {
            label,
            kind,
            x,
            y,
            w,
            h,
            members: &[],
        }
    }

    /// The same widget saving `members` with its settings.
    const fn with(mut self, members: &'static [(&'static str, &'static str)]) -> Self {
        self.members = members;
        self
    }

    /// Its settings: the kind's defaults, or what its members read as.
    fn settings(&self) -> WidgetSettings {
        if self.members.is_empty() {
            return WidgetSettings::new(self.kind);
        }
        let saved = self
            .members
            .iter()
            .map(|(name, value)| ((*name).to_owned(), Value::from(*value)))
            .collect();
        WidgetSettings::read(self.kind, Value::Object(saved))
    }
}

/// A default layout: its name, what it is for and its widgets, the first
/// on top where they are stacked.
pub(crate) struct Template {
    /// What test ids call it.
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub panels: &'static [Panel],
}

/// Every default layout, in the order the Layout Manager shows them.
pub(crate) const ALL: &[Template] = &[
    Template {
        id: "trading-desk",
        name: "Trading Desk",
        description: "A chart over market depth, beside the order book and live trades.",
        panels: &[
            Panel::new("Chart", WidgetKind::Chart, [0, 0, 6, 4]),
            Panel::new("Book", WidgetKind::OrderBook, [6, 0, 3, 6]),
            Panel::new("Trades", WidgetKind::TradeStream, [9, 0, 3, 6]),
            Panel::new("Depth", WidgetKind::DepthChart, [0, 4, 6, 2]),
        ],
    },
    Template {
        id: "market-overview",
        name: "Market Overview",
        description: "A chart, the market screener and live trades for the wider picture.",
        panels: &[
            Panel::new("Chart", WidgetKind::Chart, [0, 0, 7, 4]),
            Panel::new("Markets", WidgetKind::MarketScreener, [7, 0, 5, 6]),
            Panel::new("Trades", WidgetKind::TradeStream, [0, 4, 7, 2]),
        ],
    },
    Template {
        id: "order-flow",
        name: "Order Flow",
        description: "Order book, depth and live trades lined up side by side.",
        panels: &[
            Panel::new("Book", WidgetKind::OrderBook, [0, 0, 4, 6]),
            // Prices down the pane, level with the book's.
            Panel::new("Depth", WidgetKind::DepthChart, [4, 0, 4, 6])
                .with(&[("layout", "vertical")]),
            Panel::new("Trades", WidgetKind::TradeStream, [8, 0, 4, 6]),
        ],
    },
    Template {
        id: "multi-timeframe",
        name: "Multi-Timeframe",
        description: "One market's price action on 1m, 15m and 1h charts.",
        panels: &[
            Panel::new("1m", WidgetKind::Chart, [0, 0, 8, 6]),
            Panel::new("15m", WidgetKind::Chart, [8, 0, 4, 3]).with(&[("timeframe", "15m")]),
            Panel::new("1h", WidgetKind::Chart, [8, 3, 4, 3]).with(&[("timeframe", "1h")]),
        ],
    },
    Template {
        id: "funding-arbitrage",
        name: "Funding & Arbitrage",
        description: "A funding watchlist (choose your pairs), price deltas, the spread chart and \
                      markets.",
        panels: &[
            Panel::new("Funding", WidgetKind::FundingRates, [0, 0, 7, 3]),
            Panel::new("Deltas", WidgetKind::PriceDeltaMonitor, [7, 0, 5, 3]),
            Panel::new("Spread", WidgetKind::PremiumArbChart, [0, 3, 7, 3]),
            Panel::new("Markets", WidgetKind::MarketScreener, [7, 3, 5, 3]),
        ],
    },
    Template {
        id: "scalping",
        name: "Scalping",
        description: "The order book and live trades beside tick and 1m charts.",
        panels: &[
            Panel::new("Tick", WidgetKind::Chart, [6, 0, 6, 3]).with(&[("timeframe", "tick")]),
            Panel::new("Book", WidgetKind::OrderBook, [0, 0, 3, 6]),
            Panel::new("Trades", WidgetKind::TradeStream, [3, 0, 3, 6]),
            Panel::new("1m", WidgetKind::Chart, [6, 3, 6, 3]),
        ],
    },
];

impl Template {
    /// The grid of a page made from it, a design row `unit` grid rows high.
    /// Its widgets are new ones.
    fn grid(&self, unit: u32) -> Grid {
        let tabs: Vec<Tab> = self
            .panels
            .iter()
            .enumerate()
            .map(|(index, panel)| Tab {
                id: index + 1,
                title: panel.kind.title().into(),
                widget: WidgetId::fresh(),
                settings: TabWidget::Known(panel.settings()),
            })
            .collect();
        let mut grid = Grid::default();
        for (breakpoint, layout) in grid.layouts.iter_mut().enumerate() {
            let columns = columns(breakpoint);
            let mut stacked = 0;
            *layout = self
                .panels
                .iter()
                .zip(&tabs)
                .enumerate()
                .map(|(index, (panel, tab))| {
                    let rect = if columns <= STACKED_COLUMNS {
                        let h = (panel.h * unit).clamp(STACKED_ROWS.0, STACKED_ROWS.1);
                        let rect = Rect {
                            x: 0,
                            y: stacked,
                            w: columns,
                            h,
                        };
                        stacked += h;
                        rect
                    } else {
                        let x = scaled(panel.x, columns);
                        Rect {
                            x,
                            y: panel.y * unit,
                            w: scaled(panel.x + panel.w, columns) - x,
                            h: panel.h * unit,
                        }
                    };
                    Pane {
                        id: index + 1,
                        rect,
                        tabs: vec![tab.clone()],
                        active_tab: tab.id,
                    }
                })
                .collect();
        }
        grid.repair_counters();
        grid
    }
}

/// The grid column a design column's edge is at, of `columns`: the nearest
/// one, so two widgets that share an edge on the design share it here.
fn scaled(edge: u32, columns: u32) -> u32 {
    (edge * columns + COLUMNS / 2) / COLUMNS
}

/// The grid rows a design row takes for the design to fill a workspace
/// `height` points high.
fn unit(height: f32) -> u32 {
    let rows = ((height - GAP) / ROW_STEP).max(0.0) as u32;
    (rows / ROWS).clamp(UNIT.0, UNIT.1)
}

/// Starts a page from `template` in a workspace `height` points high: the
/// active page when it has no pane, a new page otherwise, which becomes the
/// active one, so no page loses what it had. True when the active page was
/// the one used.
pub(crate) fn apply(workspace: &mut StoredWorkspace, template: &Template, height: f32) -> bool {
    let blank = active_grid(workspace).layouts.iter().all(Vec::is_empty);
    if !blank {
        add_page(workspace);
    }
    *active_grid_mut(workspace) = template.grid(unit(height));
    blank
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::layout::{export_code, grid::overlaps, import_code, normalize, widget_ids};

    fn rects(panes: &[Pane]) -> Vec<Rect> {
        panes.iter().map(|pane| pane.rect).collect()
    }

    fn disjoint(rects: &[Rect]) -> bool {
        rects
            .iter()
            .enumerate()
            .all(|(index, rect)| rects[..index].iter().all(|other| !overlaps(*rect, *other)))
    }

    #[test]
    fn every_template_fills_the_design_with_widgets_that_do_not_overlap() {
        let mut ids = HashSet::new();
        for template in ALL {
            assert!(ids.insert(template.id), "{} is listed twice", template.id);
            let design: Vec<Rect> = template
                .panels
                .iter()
                .map(|panel| Rect {
                    x: panel.x,
                    y: panel.y,
                    w: panel.w,
                    h: panel.h,
                })
                .collect();
            assert!(disjoint(&design), "{}", template.id);
            assert!(
                design
                    .iter()
                    .all(|rect| rect.x + rect.w <= COLUMNS && rect.y + rect.h <= ROWS),
                "{}",
                template.id
            );
            let covered: u32 = design.iter().map(|rect| rect.w * rect.h).sum();
            assert_eq!(covered, COLUMNS * ROWS, "{}", template.id);
        }
    }

    #[test]
    fn a_page_made_from_a_template_has_its_widgets_at_every_breakpoint() {
        for template in ALL {
            let mut grid = template.grid(8);
            let widgets: Vec<WidgetId> = grid.layouts[4]
                .iter()
                .map(|pane| pane.tabs[0].widget)
                .collect();
            assert_eq!(
                widgets.iter().collect::<HashSet<_>>().len(),
                template.panels.len(),
                "{}",
                template.id
            );
            for (breakpoint, layout) in grid.layouts.iter().enumerate() {
                let columns = columns(breakpoint);
                let rects = rects(layout);
                assert!(disjoint(&rects), "{} at {columns}", template.id);
                assert!(rects.iter().all(|rect| rect.x + rect.w <= columns));
                // The same widgets, each of its panel's kind and titled so.
                for ((pane, panel), widget) in layout.iter().zip(template.panels).zip(&widgets) {
                    let tab = &pane.tabs[0];
                    assert_eq!((tab.widget, pane.active_tab), (*widget, tab.id));
                    assert_eq!(tab.kind(), Some(panel.kind));
                    assert_eq!(tab.title, panel.kind.title());
                }
                let bottom = rects.iter().map(|rect| rect.y + rect.h).max();
                if columns > STACKED_COLUMNS {
                    // The design, edge to edge.
                    assert_eq!(bottom, Some(ROWS * 8), "{} at {columns}", template.id);
                    let area: u32 = rects.iter().map(|rect| rect.w * rect.h).sum();
                    assert_eq!(area, columns * ROWS * 8, "{} at {columns}", template.id);
                } else {
                    assert!(rects.iter().all(|rect| rect.x == 0 && rect.w == columns));
                }
            }
            // The next pane is a new one.
            assert_eq!(
                grid.spawn(WidgetKind::MarketScreener),
                template.panels.len() + 1
            );
        }
    }

    #[test]
    fn a_templates_settings_are_what_a_saved_tab_would_hold() {
        for panel in ALL.iter().flat_map(|template| template.panels) {
            let saved = panel.settings().to_value();
            if panel.members.is_empty() {
                assert!(panel.settings().is_default(), "{}", panel.label);
            }
            for (name, value) in panel.members {
                assert_eq!(
                    saved.get(name).and_then(Value::as_str),
                    Some(*value),
                    "{} of {}",
                    name,
                    panel.label
                );
            }
        }
    }

    #[test]
    fn a_design_row_is_as_high_as_fills_the_workspace() {
        // A 1280 by 800 window's workspace: 46 rows, 7 to a design row.
        assert_eq!(unit(744.0), 7);
        assert!((ROWS * unit(744.0)) as f32 * ROW_STEP + GAP <= 744.0);
        assert_eq!(unit(984.0), 10);
        // Never fewer than a short window's, nor more than a tall one's.
        assert_eq!(unit(0.0), UNIT.0);
        assert_eq!(unit(-40.0), UNIT.0);
        assert_eq!(unit(f32::NAN), UNIT.0);
        assert_eq!(unit(100_000.0), UNIT.1);
    }

    #[test]
    fn applying_uses_a_blank_page_and_adds_one_to_keep_a_page_with_widgets() {
        let mut saved = StoredWorkspace::default();
        normalize(&mut saved);
        let (first, second) = (&ALL[0], &ALL[1]);

        assert!(apply(&mut saved, first, 744.0));
        assert_eq!((saved.pages.len(), saved.active_page_id), (1, 1));
        assert_eq!(widget_ids(&saved).len(), first.panels.len());
        let page = saved.pages[0].clone();

        // A page with widgets is kept as it is.
        assert!(!apply(&mut saved, second, 744.0));
        assert_eq!((saved.pages.len(), saved.active_page_id), (2, 2));
        assert_eq!(saved.pages[0], page);
        assert_eq!(saved.pages[1].grid.layouts[4].len(), second.panels.len());
        assert_eq!(
            widget_ids(&saved).len(),
            first.panels.len() + second.panels.len()
        );

        // What was applied saves and loads like any page.
        let code = export_code(&saved).unwrap();
        assert_eq!(export_code(&import_code(&code).unwrap()).unwrap(), code);
    }
}
