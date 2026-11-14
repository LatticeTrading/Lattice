//! The exchange tabs: "All", then each exchange with markets on offer, in
//! a strip that expands to every row.

use nsh_core::prelude::*;

use super::listing::Listing;
use crate::{
    exchanges, theme,
    widgets::common::{LogoStyle, Shared, exchange_logo},
};

/// How the exchange tabs draw logos.
const LOGO: LogoStyle = LogoStyle {
    size: 14.0,
    color: theme::TEXT,
    initials_size: 7.0,
    boxed: true,
};

/// How many tabs a row of the strip holds.
const TABS_PER_ROW: usize = 4;
/// The strip's expand button's width.
const TOGGLE_WIDTH: f32 = 34.0;

/// "All", then a tab per exchange with markets on offer, in a grid of rows
/// of [`TABS_PER_ROW`] equal tabs; a last row with fewer shares the width
/// evenly among them. Collapsed (every opening) it shows one row, the one
/// holding the selected tab, so the selection is always in view; the
/// button at its right expands it to every row and collapses it again.
pub(super) fn tabs(listing: Memo<Shared<Listing>>, tab: Signal<Option<String>>) -> View {
    let ids = computed(move || {
        listing.with(|listing| {
            std::iter::once(None)
                .chain(listing.0.exchanges.iter().map(|(id, _)| Some(id.clone())))
                .collect::<Vec<_>>()
        })
    });
    let expanded = signal(false);
    let rows = computed(move || ids.with(|ids| ids.len().div_ceil(TABS_PER_ROW)));
    // The row the selected tab is in (a tab no longer listed shows "All",
    // which is in the first).
    let selected_row = computed(move || {
        let selected = tab.get();
        ids.with(|ids| {
            ids.iter()
                .position(|id| *id == selected)
                .map_or(0, |at| at / TABS_PER_ROW)
        })
    });
    let grid = dynamic(move || {
        let ids = ids.get();
        let shown: Vec<usize> = if expanded.get() {
            (0..rows.get()).collect()
        } else {
            vec![selected_row.get()]
        };
        shown
            .into_iter()
            .enumerate()
            .map(|(at, row)| {
                let cells = ids
                    .iter()
                    .skip(row * TABS_PER_ROW)
                    .take(TABS_PER_ROW)
                    .cloned()
                    .collect::<Vec<_>>();
                let last = cells.len().saturating_sub(1);
                let line = view().flex_row().w_full();
                let line = if at > 0 {
                    line.border_t_1().border_color(theme::BORDER)
                } else {
                    line
                };
                line.children(
                    cells
                        .into_iter()
                        .enumerate()
                        .map(|(cell, id)| tab_button(id, tab, cell == last)),
                )
            })
            .collect::<Vec<_>>()
    });
    let toggle = view()
        .w(px(TOGGLE_WIDTH))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .border_l_1()
        .border_color(theme::BORDER)
        .text_size(px(11.0))
        .text_color(theme::TEXT_MUTED)
        .role(AccessibilityRole::Button)
        .label(move || {
            if expanded.get() {
                "Show fewer exchanges"
            } else {
                "Show every exchange"
            }
        })
        .accessibility_state(move || AccessibilityState {
            expanded: Some(expanded.get()),
            ..Default::default()
        })
        .hover(|s| s.bg(theme::PANEL).text_color(theme::TEXT))
        .style_when(move || rows.get() < 2, |s| s.hidden())
        .on_press(move || expanded.update(|open| *open = !*open))
        .child(text(move || if expanded.get() { "▴" } else { "▾" }));
    view()
        .flex_none()
        .flex_row()
        .bg(theme::SURFACE)
        .border_b_1()
        .border_color(theme::BORDER)
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .flex_col()
                .role(AccessibilityRole::Tablist)
                .label("Exchanges")
                .child(grid),
        )
        .child(toggle)
}

/// One exchange's tab (`None`: "All"), with its logo: an equal share of
/// its row, divided from the next unless it is the row's `last`.
fn tab_button(exchange: Option<String>, tab: Signal<Option<String>>, last: bool) -> View {
    let name = exchange
        .as_deref()
        .map_or("All", exchanges::display_name)
        .to_owned();
    let active = {
        let exchange = exchange.clone();
        move || tab.with(|tab| *tab == exchange)
    };
    let (shade, mark) = (active.clone(), active.clone());
    let mut button = view()
        .relative()
        .h(px(34.0))
        .flex_1()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .px(px(8.0))
        .text_size(px(11.0))
        .text_color(move || {
            if shade() {
                theme::TEXT.get()
            } else {
                theme::TEXT_MUTED.get()
            }
        })
        .bg(move || {
            if active() {
                theme::PANEL.get()
            } else {
                theme::SURFACE.get()
            }
        })
        .role(AccessibilityRole::Tab)
        .label(name.clone())
        .accessibility_state({
            let exchange = exchange.clone();
            move || AccessibilityState {
                selected: tab.with(|tab| *tab == exchange),
                ..Default::default()
            }
        })
        .hover(|style| style.bg(theme::PANEL).text_color(theme::TEXT));
    if !last {
        button = button.border_r_1().border_color(theme::BORDER);
    }
    if let Some(id) = &exchange {
        button = button.child(exchange_logo(id, LOGO));
    }
    button
        .on_press(move || tab.set(exchange.clone()))
        .child(text(name).min_w(px(0.0)).truncate())
        .child(
            view()
                .absolute()
                .left(px(0.0))
                .right(px(0.0))
                .bottom(px(0.0))
                .h(px(2.0))
                .bg(theme::ACCENT)
                .style_when(move || !mark(), |style| style.hidden()),
        )
}
