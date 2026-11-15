//! The order book's settings panel, laid out as the old app's
//! `SettingsPanel` drew its schema (`webGLOrderBookSettingsSchema`): a row
//! per setting, its name on the left and its control on the right, as the
//! Chart's and the Trade Stream's. The pane's gear opens it for the active
//! tab; every change is saved in the tab record at once.
//!
//! - Primary, Pair 2: each book's market in the old chooser box (logo,
//!   pair, type, detail), which opens the market picker; `Select...` while
//!   it has none. A market can be cleared with the × beside its box, or
//!   with the picker's Clear.
//! - Grouping: the old compact select (Raw, 0.01 to 100).
//! - Depth: the old compact number field, 10 to 1000 levels a side.
//! - The six switches ([`Toggle`]): the old square checkboxes.
//! - Layout: the old compact select (Vertical, Horizontal).
//!
//! The book the ladder follows is not here: it was the top bar's alone in
//! the old app too.

use nsh_base::SelectItem;
use nsh_core::prelude::*;

use super::{
    OrderBookSettings,
    settings::{BookLayout, Grouping, MAX_DEPTH, MIN_DEPTH, Pair, Toggle},
};
use crate::{
    data::ferris::{Channel, Operation, Requirement},
    theme,
    ui::{self, Side},
    widgets::{
        TabContext,
        common::{MarketPicker, market_chooser, market_name, number_field},
    },
};

/// The old panel's width.
const PANEL_WIDTH: Points = px(340.0);
/// A market's box and its × at their widest (the old
/// `.settings-symbol-search-btn`).
const CHOOSER_WIDTH: f32 = 260.0;

/// The settings panel of the order book's tab, below the pane's gear.
pub(super) fn panel(tab: TabContext<OrderBookSettings>) -> Overlay {
    // The pair the market picker is open for.
    let picking = signal(None::<Pair>);
    let picker = {
        let tab = tab.clone();
        dynamic(move || {
            picking
                .get()
                .map(|pair| market_picker(&tab, pair, move || picking.set(None)))
        })
    };
    let body = view()
        .test_id("order-book-settings")
        .flex_col()
        .py(px(4.0))
        .child(market_row(tab.clone(), Pair::Primary, picking))
        .child(market_row(tab.clone(), Pair::Secondary, picking))
        .child(grouping_row(tab.clone()))
        .child(depth_row(tab.clone()))
        .children(
            Toggle::ALL
                .into_iter()
                .map(|toggle| toggle_row(tab.clone(), toggle)),
        )
        .child(layout_row(tab.clone()))
        .child(picker);
    let close = {
        let tab = tab.clone();
        move || tab.close_settings()
    };
    ui::settings_panel(
        tab.settings_anchor(),
        Side::Right,
        PANEL_WIDTH,
        "Order Book settings",
        close,
        body,
    )
}

/// The market picker for `pair`'s market, offering the markets whose
/// exchange serves a book both ways (a snapshot and a live topic); a pick
/// or Clear is saved, and either calls `on_close`. The settings panel and
/// the top bar both open it.
pub(super) fn market_picker(
    tab: &TabContext<OrderBookSettings>,
    pair: Pair,
    on_close: impl Fn() + Clone + 'static,
) -> View {
    let saved = tab.settings;
    let current = saved.get_untracked().market(pair);
    let pick = {
        let tab = tab.clone();
        move |picked| tab.save(&saved.get_untracked().with_market(pair, Some(picked)))
    };
    let mut picker = MarketPicker::new(pick).requires(Requirement {
        snapshot: Some(Operation::FetchOrderBook),
        realtime: Some(Channel::OrderBook),
    });
    if current.is_some() {
        let tab = tab.clone();
        picker = picker.on_clear(&format!("Clear {}", pair.label()), move || {
            tab.save(&saved.get_untracked().with_market(pair, None));
        });
    }
    picker.current(current).view(on_close)
}

/// "Primary" or "Pair 2": the pair's market in its box, which opens the
/// market picker, with × beside it while it has one.
fn market_row(
    tab: TabContext<OrderBookSettings>,
    pair: Pair,
    picking: Signal<Option<Pair>>,
) -> View {
    let saved = tab.settings;
    let market = computed(move || saved.with(|settings| settings.market(pair)));
    let control = dynamic(move || {
        let market = market.get();
        let (chooser, name) = match &market {
            Some(market) => (market_chooser(market), market_name(market)),
            None => (empty_chooser(), "Select...".to_owned()),
        };
        let chooser = chooser
            .flex_shrink(1.0)
            .min_w(px(0.0))
            .role(AccessibilityRole::Button)
            .label(format!("{} {name}", pair.label()))
            .accessibility_state(move || AccessibilityState {
                expanded: Some(picking.get() == Some(pair)),
                has_popup: true,
                ..Default::default()
            })
            .test_id(&format!("order-book-market-{}", pair.id()))
            .on_press(move || picking.set(Some(pair)));
        let row = view()
            .flex_shrink(1.0)
            .min_w(px(0.0))
            .max_w(px(CHOOSER_WIDTH))
            .flex_row()
            .child(chooser);
        match market {
            Some(_) => row.child(clear(tab.clone(), pair)),
            None => row,
        }
    });
    ui::label_edge_row(pair.label()).child(control)
}

/// The chooser box of a pair with no market: `Select...` where the pair
/// would be, as the old box read.
fn empty_chooser() -> View {
    view()
        .min_h(px(28.0))
        .flex_row()
        .bg(theme::SURFACE)
        .border_1()
        .border_color(theme::BORDER)
        .relative()
        .hover(|s| s.bg(theme::PANEL).border_color(theme::ACCENT).z_index(1))
        .child(
            view()
                .w(px(24.0))
                .flex_none()
                .border_r_1()
                .border_color(theme::BORDER),
        )
        .child(
            view().flex().items_center().px(px(8.0)).child(
                text("Select...")
                    .number_of_lines(1)
                    .text_size(px(11.0))
                    .text_color(theme::TEXT),
            ),
        )
}

/// The × beside a market's box, which clears the pair's market: laid 1pt
/// over the box's right edge so the line between them stays one line, as
/// the Trade Stream's.
fn clear(tab: TabContext<OrderBookSettings>, pair: Pair) -> View {
    let saved = tab.settings;
    view()
        .w(px(29.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::SURFACE)
        .relative()
        .ml(px(-1.0))
        .border_1()
        .border_color(theme::BORDER)
        .text_size(px(11.0))
        .text_color(theme::TEXT_MUTED)
        .role(AccessibilityRole::Button)
        .label(format!("Clear {}", pair.label()))
        .hover(|s| {
            s.bg(theme::PANEL)
                .text_color(theme::TEXT)
                .border_color(theme::ACCENT)
                .z_index(1)
        })
        .on_press(move || tab.save(&saved.get_untracked().with_market(pair, None)))
        .child(text("×"))
}

/// "Grouping": raw prices, or the step levels merge into.
fn grouping_row(tab: TabContext<OrderBookSettings>) -> View {
    let saved = tab.settings;
    let select = ui::compact_dropdown(
        || {
            Grouping::ALL
                .iter()
                .map(|grouping| SelectItem::new(grouping.label(), grouping.label()))
                .collect()
        },
        move || SharedString::from(saved.with(|settings| settings.grouping().label())),
        move || saved.with(|settings| settings.grouping().label().to_owned()),
        move |label| {
            if let Some(grouping) = Grouping::from_label(label) {
                tab.save(&saved.get_untracked().with_grouping(grouping));
            }
        },
        "Grouping",
    );
    ui::label_edge_row("Grouping").child(select)
}

/// "Depth": how many levels a side shows.
fn depth_row(tab: TabContext<OrderBookSettings>) -> View {
    let saved = tab.settings;
    let field = number_field(
        move || saved.with(OrderBookSettings::depth),
        MIN_DEPTH..=MAX_DEPTH,
        1,
        move |next| tab.save(&saved.get_untracked().with_depth(next)),
    )
    .label("Depth")
    .hint("10 to 1000 levels")
    .test_id("order-book-depth");
    ui::label_edge_row("Depth").child(field)
}

/// One switch: its name and its checkbox.
fn toggle_row(tab: TabContext<OrderBookSettings>, toggle: Toggle) -> View {
    let saved = tab.settings;
    let check = ui::check_box(
        toggle.label(),
        move || saved.with(|settings| settings.shows(toggle)),
        move || {
            let settings = saved.get_untracked();
            let on = !settings.shows(toggle);
            tab.save(&settings.with_toggle(toggle, on));
        },
    );
    ui::label_edge_row(toggle.label()).child(check)
}

/// "Layout": prices down the pane, or along its bottom.
fn layout_row(tab: TabContext<OrderBookSettings>) -> View {
    let saved = tab.settings;
    let select = ui::compact_dropdown(
        || {
            BookLayout::ALL
                .iter()
                .map(|layout| SelectItem::new(layout.id(), layout.label()))
                .collect()
        },
        move || SharedString::from(saved.with(|settings| settings.layout().id())),
        move || saved.with(|settings| settings.layout().label().to_owned()),
        move |id| {
            if let Some(layout) = BookLayout::ALL.into_iter().find(|layout| layout.id() == id) {
                tab.save(&saved.get_untracked().with_layout(layout));
            }
        },
        "Layout",
    );
    ui::label_edge_row("Layout").child(select)
}
