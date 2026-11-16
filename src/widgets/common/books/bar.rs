//! The top bar of a widget that draws order books (the old
//! `WebGLOrderBookTopBar`), and the notices of indicative books under it.
//!
//! The bar, 40 points tall:
//! - a segment per book (its exchange's logo and its pair), which opens
//!   the market picker for that book; and `+` after the primary while no
//!   second pair is tracked, which opens it for one;
//! - the widget's selects ([`quick_select`]: [`grouping_select`],
//!   [`depth_select`] and its own), which the widget hides in a pane
//!   [`NARROW`] points wide or less, as the old container query hid them
//!   (the settings panel has them too).

use nsh_base::SelectItem;
use nsh_core::prelude::*;

use super::{BookSettings, DEPTH_PRESETS, Grouping, Layer, Pair, panel::market_picker};
use crate::{
    data::ferris::{AmountUnit, BookTraits, Liquidity},
    exchanges, theme, ui,
    widgets::{
        TabContext,
        common::{LogoStyle, exchange_logo, tag_text},
    },
};

/// The top bar's height (the old `.webgl-orderbook__top-bar`).
const BAR_HEIGHT: f32 = 40.0;
/// At or under this width the top bar's selects hide.
pub(crate) const NARROW: f32 = 500.0;
/// How the top bar draws exchange logos.
const LOGO: LogoStyle = LogoStyle {
    size: 14.0,
    color: theme::TEXT,
    initials_size: 7.0,
    boxed: true,
};

/// The top bar, named `label` for assistive tech: the segments of
/// `layers`' books, then `selects` (which hide while the pane is narrow).
/// A segment sets `picking` to its pair: the widget shows the market
/// picker for it ([`picker`]).
pub(crate) fn top_bar<S: BookSettings>(
    label: &'static str,
    layers: Memo<Vec<Layer>>,
    picking: Signal<Option<Pair>>,
    narrow: Memo<bool>,
    selects: View,
) -> View {
    let pairs = view()
        .h_full()
        .min_w(px(0.0))
        .flex_row()
        .child(dynamic(move || {
            layers.with(|layers| {
                let tracks = |pair: Pair| layers.iter().any(|layer| layer.pair == pair);
                let segments = layers
                    .iter()
                    .map(|layer| segment(S::TEST_ID, layer, picking))
                    .fold(view().h_full().flex_row(), View::child);
                if tracks(Pair::Primary) && !tracks(Pair::Secondary) {
                    segments.child(track_another(S::TEST_ID, picking))
                } else {
                    segments
                }
            })
        }));
    let selects = selects
        .h_full()
        .flex_none()
        .flex_row()
        .style_when(move || narrow.get(), |s| s.hidden());
    view()
        .h(px(BAR_HEIGHT))
        .w_full()
        .flex_none()
        .flex_row()
        .justify_between()
        .bg(theme::SURFACE)
        .border_b_1()
        .border_color(theme::BORDER)
        .text_color(theme::TEXT)
        .label(label)
        .child(pairs)
        .child(selects)
}

/// The market picker of the pair `picking` names, while it names one: a
/// pick or Clear is saved in `tab`'s settings, and either lets go of the
/// pair.
pub(crate) fn picker<S: BookSettings>(
    tab: &TabContext<S>,
    picking: Signal<Option<Pair>>,
) -> impl IntoElement + use<S> {
    let tab = tab.clone();
    dynamic(move || {
        picking
            .get()
            .map(|pair| market_picker(&tab, pair, move || picking.set(None)))
    })
}

/// `Bybit BTC/USDT`: a book's exchange and pair.
pub(crate) fn book_name(layer: &Layer) -> String {
    format!(
        "{} {}",
        exchanges::display_name(&layer.market.exchange),
        layer.market.symbol
    )
}

/// A book's logo and pair.
pub(crate) fn logo_and_pair(layer: &Layer) -> View {
    view()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .gap(px(7.0))
        .child(exchange_logo(&layer.market.exchange, LOGO))
        .child(
            text(layer.market.symbol.clone())
                .min_w(px(0.0))
                .truncate()
                .text_size(px(12.0)),
        )
}

/// One book's segment of the top bar, which opens the market picker for
/// it.
fn segment(prefix: &str, layer: &Layer, picking: Signal<Option<Pair>>) -> View {
    let (pair, name) = (layer.pair, book_name(layer));
    let hint = name.clone();
    logo_and_pair(layer)
        .w(px(105.0))
        .h_full()
        .flex_none()
        .overflow_hidden()
        .px(px(8.0))
        .border_r_1()
        .border_color(theme::BORDER)
        .text_size(px(11.0))
        .role(AccessibilityRole::Button)
        .label(format!("Change {}: {name}", pair.label()))
        .test_id(&format!("{prefix}-pair-{}", pair.id()))
        .tooltip(ui::hint(move || hint.clone()))
        .hover(|s| s.bg(theme::PANEL))
        .active(|s| s.bg(theme::BORDER))
        .on_press(move || picking.set(Some(pair)))
}

/// `+`: opens the market picker for a second pair.
fn track_another(prefix: &str, picking: Signal<Option<Pair>>) -> View {
    view()
        .w(px(BAR_HEIGHT))
        .h_full()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .border_r_1()
        .border_color(theme::BORDER)
        .text_size(px(16.0))
        .text_color(theme::TEXT_FAINT)
        .role(AccessibilityRole::Button)
        .label("Track another pair")
        .test_id(&format!("{prefix}-track-pair"))
        .tooltip(ui::hint(|| "Track another pair".to_owned()))
        .hover(|s| s.bg(theme::PANEL))
        .active(|s| s.bg(theme::BORDER))
        .on_press(move || picking.set(Some(Pair::Secondary)))
        .child(text("+"))
}

/// One of the top bar's selects: its name in faint text, then what it
/// shows, which opens its list (the old `OrderBookDropdown`).
pub(crate) fn quick_select(
    name: &'static str,
    width: f32,
    shown: impl IntoElement,
    items: impl Fn() -> Vec<SelectItem> + 'static,
    value: impl Fn() -> SharedString + 'static,
    on_pick: impl Fn(&str) + 'static,
) -> impl IntoElement {
    let trigger = view()
        .size_full()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(5.0))
        .px(px(8.0))
        .overflow_hidden()
        .text_size(px(11.0))
        .text_color(theme::TEXT)
        .hover(|s| s.bg(theme::PANEL))
        .active(|s| s.bg(theme::BORDER))
        .child(
            text(name)
                .flex_none()
                .number_of_lines(1)
                .text_color(theme::TEXT_FAINT),
        )
        .child(shown);
    ui::select_from(items, value, on_pick, name, trigger)
        .w(px(width))
        .h_full()
        .flex_none()
        .border_l_1()
        .border_color(theme::BORDER)
}

/// What a select shows when it is a word or a number: on one line, at the
/// trigger's right edge.
pub(crate) fn shown_text(content: impl Fn() -> String + 'static) -> Text {
    text(content)
        .min_w(px(25.0))
        .text_right()
        .number_of_lines(1)
        .tabular_nums()
}

/// "Group": raw prices, or the step levels merge into.
pub(crate) fn grouping_select<S: BookSettings>(tab: TabContext<S>) -> impl IntoElement {
    let saved = tab.settings;
    let grouping = computed(move || saved.with(|settings| settings.books().grouping()));
    quick_select(
        "Group",
        120.0,
        shown_text(move || grouping.get().label().to_owned()),
        || {
            Grouping::ALL
                .iter()
                .map(|grouping| SelectItem::new(grouping.label(), grouping.label()))
                .collect()
        },
        move || SharedString::from(grouping.get().label()),
        move |label| {
            if let Some(grouping) = Grouping::from_label(label) {
                tab.save(&saved.get_untracked().with_grouping(grouping));
            }
        },
    )
}

/// "Depth": the levels a side, from the old presets (and the depth set in
/// the settings panel, when it is none of them).
pub(crate) fn depth_select<S: BookSettings>(tab: TabContext<S>) -> impl IntoElement {
    let saved = tab.settings;
    let depth = computed(move || saved.with(|settings| settings.books().depth()));
    quick_select(
        "Depth",
        120.0,
        shown_text(move || depth.get().to_string()),
        move || {
            let depth = depth.get();
            std::iter::once(depth)
                .filter(|depth| !DEPTH_PRESETS.contains(depth))
                .chain(DEPTH_PRESETS)
                .map(|depth| SelectItem::new(depth.to_string(), depth.to_string()))
                .collect()
        },
        move || SharedString::from(depth.get().to_string()),
        move |picked| {
            if let Ok(depth) = picked.parse::<u16>() {
                tab.save(&saved.get_untracked().with_depth(depth));
            }
        },
    )
}

/// The tags a book's [`BookTraits`] earn it, each with its tooltip: an
/// indicative book (maker quotes on an RFQ market) or one whose liquidity
/// Ferris could not classify, and, where the widget reads sizes (`units`),
/// amounts Ferris could not give in `base`.
fn tags(traits: &BookTraits, base: &str, units: bool) -> Vec<(&'static str, String)> {
    let mut tags = Vec::new();
    match traits.liquidity {
        Some(Liquidity::Indicative) => tags.push((
            "Indicative book",
            "This RFQ market's public book is maker quotes, not resting orders: \
             book-derived prices are not execution quotes."
                .to_owned(),
        )),
        Some(Liquidity::Unknown) => tags.push((
            "Unverified liquidity",
            "Whether this market's public book is resting orders or RFQ quotes \
             could not be confirmed."
                .to_owned(),
        )),
        Some(Liquidity::OrderBook) | None => {}
    }
    if units {
        let unit = match traits.unit {
            Some(AmountUnit::Contracts) => Some((
                "Contract quantities",
                format!(
                    "Sizes are the exchange's contracts, not {base}: their contract size \
                     could not be verified."
                ),
            )),
            Some(AmountUnit::Quote) => Some((
                "Quote amounts",
                format!(
                    "Sizes are in {}, not {base}.",
                    traits.currency.as_deref().unwrap_or("the quote currency")
                ),
            )),
            Some(AmountUnit::Unknown) => Some((
                "Unknown units",
                format!(
                    "The exchange does not say what these sizes are in; they may not be {base}."
                ),
            )),
            Some(AmountUnit::Base) | None => None,
        };
        tags.extend(unit);
    }
    tags
}

/// A strip under the top bar naming each book that cannot be read at face
/// value (the old `OrderBookNotice`), as what its newest state says
/// ([`BookTraits`], by its layer's key in `traits`): its [`tags`], each
/// explained by its tooltip. `units`: the widget reads the books' sizes,
/// not only their prices. `prefix` starts its test id.
pub(crate) fn notices(
    prefix: &str,
    layers: Memo<Vec<Layer>>,
    traits: Signal<Vec<(String, BookTraits)>>,
    units: bool,
) -> View {
    let noted = computed(move || {
        layers.with(|layers| {
            traits.with(|traits| {
                layers
                    .iter()
                    .filter_map(|layer| {
                        let (_, said) = traits.iter().find(|(key, _)| *key == layer.key)?;
                        let tags = tags(said, &layer.market.base, units);
                        (!tags.is_empty()).then(|| {
                            (
                                format!("{}: {}", layer.pair.label(), book_name(layer)),
                                tags,
                            )
                        })
                    })
                    .collect::<Vec<_>>()
            })
        })
    });
    let strip = || view().flex_row().flex_wrap().items_center().gap(px(8.0));
    strip()
        .flex_none()
        .px(px(8.0))
        .py(px(4.0))
        .border_b_1()
        .border_color(theme::BORDER)
        .bg(theme::PANEL)
        .text_size(px(9.0))
        .text_color(theme::TEXT_MUTED)
        .test_id(&format!("{prefix}-notices"))
        .style_when(move || noted.with(Vec::is_empty), |s| s.hidden())
        .child(dynamic(move || {
            noted.with(|noted| {
                noted
                    .iter()
                    .map(|(book, tags)| {
                        let named = view()
                            .flex_none()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .child(text(book.clone()).number_of_lines(1));
                        tags.iter()
                            .map(|(tag, hint)| {
                                let hint = hint.clone();
                                tag_text(text(*tag))
                                    .flex_none()
                                    .px(px(5.0))
                                    .py(px(1.0))
                                    .border_1()
                                    .border_color(theme::BORDER)
                                    .bg(theme::BACKGROUND)
                                    .role(AccessibilityRole::Text)
                                    .label(format!("{tag}: {hint}"))
                                    .tooltip(ui::hint(move || hint.clone()))
                            })
                            .fold(named, View::child)
                    })
                    .fold(strip(), View::child)
            })
        }))
}
