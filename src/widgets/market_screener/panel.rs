//! The screener's settings panel (the old app's `MarketScreenerSettings`):
//! which exchange scope it shows, which products, and which columns. The
//! pane's gear opens it for the active tab; every change is saved in the
//! tab record at once.

use std::rc::Rc;

use nsh_base::SelectItem;
use nsh_core::{Checked, prelude::*};

use super::{ScreenerSettings, columns::METRICS, session::Screener, settings::PRODUCT_CHOICES};
use crate::{
    data::ferris::Product,
    theme,
    ui::{self, Side, Tone},
    widgets::TabContext,
};

const PANEL_WIDTH: Points = px(300.0);
/// What the scope dropdown saves for every exchange.
const ALL: &str = "all";

/// The settings panel of `screener`'s tab, below the pane's gear.
pub(super) fn panel(screener: Rc<Screener>, tab: TabContext<ScreenerSettings>) -> Overlay {
    let body = view()
        .test_id("screener-settings")
        .flex_col()
        .child(scope_row(screener.clone(), tab.clone()))
        .child(
            products_row(screener.clone(), tab.clone())
                .border_t_1()
                .border_color(theme::BORDER),
        )
        .child(columns_list(screener, tab.clone()));
    let close = {
        let tab = tab.clone();
        move || tab.close_settings()
    };
    ui::floating_panel(
        tab.settings_anchor(),
        Side::Right,
        PANEL_WIDTH,
        "Market Screener settings",
        close,
        body,
    )
}

/// "Exchange": every exchange, or one scope of the products shown.
fn scope_row(screener: Rc<Screener>, tab: TabContext<ScreenerSettings>) -> View {
    let saved = tab.settings;
    let items = {
        let screener = screener.clone();
        move || {
            let mut items = vec![SelectItem::new(ALL, "All exchanges")];
            items.extend(
                screener
                    .choices()
                    .into_iter()
                    .map(|choice| SelectItem::new(choice.id, choice.label)),
            );
            items
        }
    };
    // A saved scope the list does not have (no longer offered) shows as
    // every exchange, as the old app did.
    let current = {
        let screener = screener.clone();
        move || {
            let scope = saved.with(|s| s.scope().map(str::to_owned));
            scope.and_then(|scope| {
                screener
                    .choices()
                    .into_iter()
                    .find(|choice| choice.id == scope)
            })
        }
    };
    let value = {
        let current = current.clone();
        move || SharedString::from(current().map_or_else(|| ALL.to_owned(), |c| c.id))
    };
    let shown = move || current().map_or_else(|| "All exchanges".to_owned(), |c| c.label);
    let picker = ui::dropdown(
        items,
        value,
        shown,
        move |id| {
            let scope = (id != ALL).then(|| id.to_owned());
            tab.save(&saved.get_untracked().with_scope(scope));
        },
        "Exchange",
    );
    ui::setting_row("Exchange").child(picker)
}

/// "Products": perpetuals and futures, each shown or not (none is a real
/// choice: the screener then shows no rows).
fn products_row(screener: Rc<Screener>, tab: TabContext<ScreenerSettings>) -> View {
    let saved = tab.settings;
    let chips = PRODUCT_CHOICES.into_iter().map(move |(product, name)| {
        let shown = move || saved.with(|s| s.products().contains(&product));
        let tab = tab.clone();
        let screener = screener.clone();
        ui::button(name, Tone::Normal)
            .flex_1()
            .justify_center()
            .border_l_1()
            .border_color(theme::BORDER)
            .label(format!("Show {name}"))
            .accessibility_state(move || AccessibilityState {
                pressed: Some(if shown() {
                    Checked::True
                } else {
                    Checked::False
                }),
                ..Default::default()
            })
            .text_color(move || {
                if shown() {
                    theme::ACCENT.get()
                } else {
                    theme::TEXT_MUTED.get()
                }
            })
            .on_press(move || {
                let settings = saved.get_untracked();
                let mut products = settings.products();
                if products.contains(&product) {
                    products.retain(|&p| p != product);
                } else {
                    products.push(product);
                }
                let mut next = settings.with_products(products.clone());
                // A scope of a product no longer shown goes back to every
                // exchange.
                let hidden = settings.scope().is_some_and(|scope| {
                    screener
                        .offered
                        .with_untracked(|offered| {
                            offered.as_ref().and_then(|offered| {
                                offered
                                    .iter()
                                    .find(|choice| choice.id == scope)
                                    .map(|choice| choice.product)
                            })
                        })
                        .is_some_and(|scope: Product| !products.contains(&scope))
                });
                if hidden {
                    next = next.with_scope(None);
                }
                // The default sort depends on the products shown.
                screener.sort.set(None);
                tab.save(&next);
            })
    });
    ui::setting_row("Products").children(chips)
}

/// "Columns": every column the screener can show besides exchange, symbol
/// and quote, each checked while shown. The last one shown cannot be
/// unchecked.
fn columns_list(screener: Rc<Screener>, tab: TabContext<ScreenerSettings>) -> View {
    let saved = tab.settings;
    let rows = METRICS.into_iter().map(move |column| {
        let checked = move || saved.with(|s| s.metrics().contains(&column));
        let last = move || saved.with(|s| s.metrics() == [column]);
        let (tab, screener) = (tab.clone(), screener.clone());
        ui::check_row(column.title(), checked, last, move || {
            let settings = saved.get_untracked();
            let Some(next) = settings.toggled(column) else {
                return;
            };
            // Sorting by a column that goes away falls back to the default.
            let hidden = !next.metrics().contains(&column);
            if hidden
                && screener
                    .sort
                    .get_untracked()
                    .is_some_and(|sort| sort.column == column)
            {
                screener.sort.set(None);
            }
            tab.save(&next);
        })
    });
    view()
        .flex_col()
        .border_t_1()
        .border_color(theme::BORDER)
        .child(
            view()
                .h(px(28.0))
                .flex()
                .items_center()
                .px(px(10.0))
                .text_size(px(11.0))
                .text_color(theme::TEXT_MUTED)
                .role(AccessibilityRole::Header)
                .child(text("Columns")),
        )
        .child(
            scroll()
                .max_h(px(360.0))
                .label("Columns")
                .child(view().flex_col().pb(px(4.0)).children(rows)),
        )
}
