//! The rows of a settings panel that its books are set with, as the old
//! app's `SettingsPanel` drew them (`webGLOrderBookSettingsSchema`), and
//! the market picker they and the top bar open:
//!
//! - Primary, Pair 2 ([`market_row`]): each book's market in the old
//!   chooser box (logo, pair, type, detail), which opens the market
//!   picker; `Select...` while it has none. A market can be cleared with
//!   the × beside its box, or with the picker's Clear.
//! - Grouping ([`grouping_row`]): the old compact select (Raw, 0.01 to
//!   100).
//! - Depth ([`depth_row`]): the old compact number field, 10 to 1000
//!   levels a side.

use nsh_base::SelectItem;
use nsh_core::prelude::*;

use super::{BookSettings, Grouping, MAX_DEPTH, MIN_DEPTH, Pair};
use crate::{
    data::ferris::{Channel, MarketSelection, Operation, Requirement},
    theme, ui,
    widgets::{
        TabContext,
        common::{MarketPicker, market_chooser, market_name, number_field},
    },
};

/// A market's box and its × at their widest (the old
/// `.settings-symbol-search-btn`).
const CHOOSER_WIDTH: f32 = 260.0;

/// The market picker for `pair`'s market, offering the markets whose
/// exchange serves a book both ways (a snapshot and a live topic); a pick
/// or Clear is saved, and either calls `on_close`. The settings panel and
/// the top bar both open it.
pub(crate) fn market_picker<S: BookSettings>(
    tab: &TabContext<S>,
    pair: Pair,
    on_close: impl Fn() + Clone + 'static,
) -> View {
    let saved = tab.settings;
    let current = saved.get_untracked().books().market(pair);
    let pick = {
        let tab = tab.clone();
        move |picked| save_market(&tab, pair, Some(picked))
    };
    let mut picker = MarketPicker::new(pick).requires(Requirement {
        snapshot: Some(Operation::FetchOrderBook),
        realtime: Some(Channel::OrderBook),
    });
    if current.is_some() {
        let tab = tab.clone();
        picker = picker.on_clear(&format!("Clear {}", pair.label()), move || {
            save_market(&tab, pair, None);
        });
    }
    picker.current(current).view(on_close)
}

/// Saves `market` as `pair`'s (`None`: no market) in `tab`'s settings.
fn save_market<S: BookSettings>(tab: &TabContext<S>, pair: Pair, market: Option<MarketSelection>) {
    tab.save(&tab.settings.get_untracked().with_market(pair, market));
}

/// "Primary" or "Pair 2": the pair's market in its box, which opens the
/// market picker (by setting `picking`), with × beside it while it has
/// one.
pub(crate) fn market_row<S: BookSettings>(
    tab: TabContext<S>,
    pair: Pair,
    picking: Signal<Option<Pair>>,
) -> View {
    let saved = tab.settings;
    let market = computed(move || saved.with(|settings| settings.books().market(pair)));
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
            .test_id(&format!("{}-market-{}", S::TEST_ID, pair.id()))
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
fn clear<S: BookSettings>(tab: TabContext<S>, pair: Pair) -> View {
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
        .on_press(move || save_market(&tab, pair, None))
        .child(text("×"))
}

/// "Grouping": raw prices, or the step levels merge into.
pub(crate) fn grouping_row<S: BookSettings>(tab: TabContext<S>) -> View {
    let saved = tab.settings;
    let grouping = move || saved.with(|settings| settings.books().grouping());
    let select = ui::compact_dropdown(
        || {
            Grouping::ALL
                .iter()
                .map(|grouping| SelectItem::new(grouping.label(), grouping.label()))
                .collect()
        },
        move || SharedString::from(grouping().label()),
        move || grouping().label().to_owned(),
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
pub(crate) fn depth_row<S: BookSettings>(tab: TabContext<S>) -> View {
    let saved = tab.settings;
    let field = number_field(
        move || saved.with(|settings| settings.books().depth()),
        MIN_DEPTH..=MAX_DEPTH,
        1,
        move |next| tab.save(&saved.get_untracked().with_depth(next)),
    )
    .label("Depth")
    .hint("10 to 1000 levels")
    .test_id(&format!("{}-depth", S::TEST_ID));
    ui::label_edge_row("Depth").child(field)
}
