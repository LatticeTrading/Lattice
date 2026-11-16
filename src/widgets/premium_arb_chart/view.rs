//! The Premium/Arb Chart's view, top to bottom:
//!
//! - the notices of indicative books every widget reading books has
//!   ([`books::notices`]), naming each market whose exchange says its
//!   public book is not what it seems;
//! - with both markets set, the plot, which is the shared one
//!   ([`plot_view`]: the line on shader tiles, the axes, the newest
//!   difference's tag, the status, the crosshair and the pointer's
//!   handling, all as the Chart's), with the widget's header over it: what
//!   is compared, as the old chart's header read
//!   (`BTC/USDT  ·  Tick  ·  Raw Delta | Binance vs Bybit`);
//! - with one missing, the old chooser instead: a button per market, each
//!   half the widget, which opens the market picker for it.

use std::rc::Rc;

use nsh_core::prelude::*;

use super::{
    PremiumArbSettings,
    panel::{self, pair_name},
    session::PremiumArbSession,
    settings::Display,
};
use crate::{
    data::ferris::MarketSelection,
    exchanges, theme,
    widgets::{
        TabContext,
        common::{
            books::{self, Pair},
            market_tags,
            plot::{self, PlotLook, plot_view},
        },
    },
};

/// What its test ids start with: the old app's widget id.
const ID: &str = "premium-arb-chart";

/// The Premium/Arb Chart of `session`, as its tab's settings say.
pub(super) fn premium_arb_view(
    session: &Rc<PremiumArbSession>,
    tab: &TabContext<PremiumArbSettings>,
) -> View {
    let saved = tab.settings;
    let layers = computed(move || saved.with(PremiumArbSettings::layers));
    let paired = computed(move || saved.with(PremiumArbSettings::paired));
    // The pair the chooser's market picker is open for.
    let picking = signal(None::<Pair>);
    // A tab that no longer shows closes it, as the old widget did.
    let shown = tab.shown;
    effect(move || {
        if !shown.get() {
            picking.set(None);
        }
    });
    view()
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .flex_col()
        .overflow_hidden()
        .bg(theme::BACKGROUND)
        .text_color(theme::TEXT)
        .test_id(ID)
        .child(books::notices(ID, layers, session.traits, false))
        // Both stay mounted, so the plot keeps its size and its pointer
        // state while a market is picked.
        .child(plot(session, saved).style_when(move || !paired.get(), |s| s.hidden()))
        .child(chooser(saved, picking).style_when(move || paired.get(), |s| s.hidden()))
        .child(panel::picker(tab, picking))
}

/// The plot, with the header over it.
fn plot(session: &Rc<PremiumArbSession>, saved: Memo<PremiumArbSettings>) -> View {
    let status = session.status;
    let look = PlotLook {
        id: ID,
        grid: computed(move || saved.with(PremiumArbSettings::grid)),
        // A point is one value, which the crosshair's tag on the axis
        // names: the old chart showed no box of a bar's prices either.
        bar_prices: computed(|| false),
        status: computed(move || status.get().text()),
        pointer: None,
        legend: computed(|| 0.0),
    };
    let header = plot::header(move || {
        saved.with(|settings| {
            settings
                .markets()
                .map_or_else(String::new, |(primary, secondary)| {
                    title(&primary, &secondary, settings.display())
                })
        })
    })
    .test_id(&format!("{ID}-header"));
    plot_view(
        &session.plot,
        look,
        view().absolute().size_full().child(header),
    )
}

/// What the plot's header says (the old chart's): the pair when both
/// markets trade the same one, else the second's and the primary's; that
/// its points are ticks; and what is compared, the second market's
/// exchange against the primary's.
pub(super) fn title(
    primary: &MarketSelection,
    secondary: &MarketSelection,
    display: Display,
) -> String {
    let symbol = if primary.symbol == secondary.symbol {
        primary.symbol.clone()
    } else {
        format!("{} - {}", secondary.symbol, primary.symbol)
    };
    format!(
        "{symbol}  ·  Tick  ·  {} | {} vs {}",
        display.label(),
        exchanges::display_name(&secondary.exchange),
        exchanges::display_name(&primary.exchange)
    )
}

/// The old chooser, shown while a market is missing: a button per market,
/// the primary's over the second's.
fn chooser(saved: Memo<PremiumArbSettings>, picking: Signal<Option<Pair>>) -> View {
    view()
        .flex_1()
        .min_h(px(0.0))
        .flex_col()
        .child(chooser_button(saved, Pair::Primary, picking))
        .child(
            chooser_button(saved, Pair::Secondary, picking)
                .border_t_1()
                .border_color(theme::BORDER),
        )
}

/// One market's half of the chooser (the old
/// `.premium-arb-chart__chooser-button`): its name, then the market set
/// (its exchange and pair, and its tags) or what to pick. Pressing it opens
/// the market picker for it.
fn chooser_button(
    saved: Memo<PremiumArbSettings>,
    pair: Pair,
    picking: Signal<Option<Pair>>,
) -> View {
    let market = computed(move || saved.with(|settings| settings.market(pair)));
    let set = move || market.with(Option::is_some);
    let of = move |read: fn(&MarketSelection) -> Option<String>| {
        market.with(|market| market.as_ref().and_then(read))
    };
    let empty = match pair {
        Pair::Primary => "Select primary pair",
        Pair::Secondary => "Select second pair",
    };
    view()
        .flex_1()
        .min_h(px(0.0))
        .w_full()
        .flex_col()
        .justify_center()
        .gap(px(6.0))
        .p(px(16.0))
        .style_when(set, |s| s.bg(theme::ACCENT.opacity(0.025)))
        .hover(|s| s.bg(theme::ACCENT.opacity(0.05)))
        .role(AccessibilityRole::Button)
        .label(format!("Select {}", pair_name(pair)))
        .test_id(&format!("{ID}-pick-{}", pair.id()))
        .on_press(move || picking.set(Some(pair)))
        .child(
            text(pair_name(pair))
                .number_of_lines(1)
                .text_size(px(10.0))
                .font_weight(FontWeight::SEMIBOLD)
                .letter_spacing(px(0.8))
                .uppercase()
                .text_color(theme::TEXT_FAINT),
        )
        .child(
            text(move || {
                of(|market| {
                    Some(format!(
                        "{} {}",
                        exchanges::display_name(&market.exchange),
                        market.symbol
                    ))
                })
                .unwrap_or_else(|| empty.to_owned())
            })
            .number_of_lines(1)
            .text_size(px(15.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme::TEXT),
        )
        .child(
            market_tags(
                move || of(|market| Some(market.kind.label().to_owned())).unwrap_or_default(),
                move || of(MarketSelection::detail),
            )
            .style_when(move || !set(), |s| s.hidden()),
        )
}
