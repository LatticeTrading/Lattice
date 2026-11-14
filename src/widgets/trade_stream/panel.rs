//! The Trade Stream's settings panel, laid out as the old app's
//! `SettingsPanel` drew the stream's schema (`SettingsPanel.css`): a row
//! per setting, its name on the left and its control on the right. The
//! pane's gear opens it for the active tab; every change is saved in the
//! tab record at once.
//!
//! - Markets (the old `symbol-search-list`): a bordered box per market,
//!   in segments (logo, pair, type, detail), with its × beside it; pressing
//!   the box opens the market picker to replace that market, × removes it
//!   (the last one too: the stream then asks for a pair). "Add pair", a
//!   dashed box under them, opens the picker to add one; a market already
//!   watched is not added twice ([`TradeStreamSettings::with_markets`]).
//!   The picker offers markets whose exchange streams `trades` for their
//!   product; REST history is an optional head start, so it is not
//!   required (Perpl has none).
//! - Max Trades: the old compact number field, 60pt wide and right
//!   aligned. A number is kept on Enter or on leaving the field, clamped
//!   to 10-200; anything else goes back. Up and Down step it by 10 while it
//!   has the focus, as the old field's did.

use std::rc::Rc;

use nsh_core::prelude::*;

use super::{
    TradeStreamSettings,
    settings::{MAX_TRADES, MIN_TRADES},
};
use crate::{
    data::ferris::{Channel, MarketSelection, Requirement},
    exchanges, theme,
    ui::{self, Side},
    widgets::{
        TabContext,
        common::{LogoStyle, MarketPicker, exchange_logo, number_field},
    },
};

/// The old panel's width.
const PANEL_WIDTH: Points = px(340.0);
/// What Up and Down change Max Trades by (the old field's step).
const STEP: u16 = 10;
/// The market list's widest (the old `.settings-symbol-list`).
const LIST_WIDTH: f32 = 260.0;
/// A market box's height, and its logo segment's width.
const ENTRY_HEIGHT: f32 = 28.0;
const LOGO_SEGMENT: f32 = 24.0;
/// How a market's box draws its exchange's logo: initials without a box,
/// the segment is one.
const LOGO: LogoStyle = LogoStyle {
    size: 14.0,
    color: theme::TEXT_MUTED,
    initials_size: 7.0,
    boxed: false,
};

/// What the market picker is open for.
#[derive(Clone, Debug, PartialEq)]
enum Picking {
    /// A market to add.
    Add,
    /// A market to put in place of this one.
    Replace(Rc<MarketSelection>),
}

/// The settings panel of the stream's tab, below the pane's gear.
pub(super) fn panel(tab: TabContext<TradeStreamSettings>) -> Overlay {
    let picking = signal(None::<Picking>);
    let picker = {
        let tab = tab.clone();
        dynamic(move || {
            picking
                .get()
                .map(|picking_for| picker(picking_for, &tab, picking))
        })
    };
    let body = view()
        .test_id("trade-stream-settings")
        .flex_col()
        .py(px(4.0))
        .child(markets(&tab, picking))
        .child(max_trades(tab.clone()))
        .child(picker);
    let close = {
        let tab = tab.clone();
        move || tab.close_settings()
    };
    ui::settings_panel(
        tab.settings_anchor(),
        Side::Right,
        PANEL_WIDTH,
        "Trade Stream settings",
        close,
        body,
    )
}

/// The market picker, open for `picking_for`; a pick is saved and closes it.
fn picker(
    picking_for: Picking,
    tab: &TabContext<TradeStreamSettings>,
    picking: Signal<Option<Picking>>,
) -> View {
    let saved = tab.settings;
    let current = match &picking_for {
        Picking::Add => None,
        Picking::Replace(market) => Some(MarketSelection::clone(market)),
    };
    let tab = tab.clone();
    MarketPicker::new(move |picked| {
        let settings = saved.get_untracked();
        let mut markets = settings.markets();
        match &picking_for {
            Picking::Add => markets.push(picked),
            Picking::Replace(replaced) => {
                let identity = replaced.identity();
                match markets
                    .iter()
                    .position(|market| market.identity() == identity)
                {
                    Some(at) => markets[at] = picked,
                    None => markets.push(picked),
                }
            }
        }
        // A market already watched is kept once, where it was.
        tab.save(&settings.with_markets(markets));
    })
    .requires(Requirement::realtime(Channel::Trades))
    .current(current)
    .view(move || picking.set(None))
}

/// "Markets": a box per market watched, then "Add pair".
fn markets(tab: &TabContext<TradeStreamSettings>, picking: Signal<Option<Picking>>) -> View {
    let saved = tab.settings;
    let watched = computed(move || saved.with(TradeStreamSettings::markets));
    let entries = {
        let tab = tab.clone();
        view()
            .flex_col()
            .gap(px(4.0))
            .role(AccessibilityRole::List)
            .label("Watched markets")
            .style_when(move || watched.with(Vec::is_empty), |s| s.hidden())
            .child(each(
                move || watched.get(),
                MarketSelection::identity,
                move |market, _| market_entry(market.get_untracked(), tab.clone(), picking),
            ))
    };
    let add = view()
        .min_h(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .px(px(8.0))
        .border_1()
        .border_dashed()
        .border_color(theme::BORDER)
        .text_size(px(11.0))
        .text_color(theme::TEXT_MUTED)
        .role(AccessibilityRole::Button)
        .label("Add pair to Markets")
        .test_id("trade-stream-add-pair")
        .hover(|s| s.border_color(theme::ACCENT).text_color(theme::TEXT))
        .on_press(move || picking.set(Some(Picking::Add)))
        .child(text("Add pair"));
    ui::form_list_row("Markets").child(
        view()
            .min_w(px(0.0))
            .max_w(px(LIST_WIDTH))
            .flex_shrink(1.0)
            .flex_col()
            .gap(px(4.0))
            .child(entries)
            .child(add),
    )
}

/// One watched market: its box (logo, pair, type, detail) replaces it
/// when pressed, and × beside it removes it.
fn market_entry(
    market: MarketSelection,
    tab: TabContext<TradeStreamSettings>,
    picking: Signal<Option<Picking>>,
) -> View {
    let saved = tab.settings;
    let name = format!(
        "{} {} {}",
        exchanges::display_name(&market.exchange),
        market.symbol,
        market.tags()
    );
    let identity = market.identity();
    let replacing = {
        let identity = identity.clone();
        move || {
            picking.with(
                |picking| matches!(picking, Some(Picking::Replace(m)) if m.identity() == identity),
            )
        }
    };
    // The old `.market-tag` segments: no box of their own, a divider
    // before each.
    let segment = |label: String, color: theme::Role| {
        view()
            .flex_none()
            .flex()
            .items_center()
            .px(px(8.0))
            .border_l_1()
            .border_color(theme::BORDER)
            .child(
                text(label)
                    .number_of_lines(1)
                    .text_size(px(9.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .letter_spacing(px(0.4))
                    .uppercase()
                    .text_color(color),
            )
    };
    let mut chooser = view()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(ENTRY_HEIGHT))
        .flex_row()
        .bg(theme::SURFACE)
        .border_1()
        .border_color(theme::BORDER)
        .role(AccessibilityRole::Button)
        .label(format!("Replace {name}"))
        .accessibility_state(move || AccessibilityState {
            expanded: Some(replacing()),
            ..Default::default()
        })
        // Raised while hovered, so its accent edge draws over the × box's
        // (the two share their dividing line).
        .relative()
        .hover(|s| s.bg(theme::PANEL).border_color(theme::ACCENT).z_index(1))
        .on_press({
            let picked = Rc::new(market.clone());
            move || picking.set(Some(Picking::Replace(picked.clone())))
        })
        .child(
            view()
                .w(px(LOGO_SEGMENT))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .border_r_1()
                .border_color(theme::BORDER)
                .overflow_hidden()
                .child(exchange_logo(&market.exchange, LOGO)),
        )
        .child(
            view()
                .flex_shrink(1.0)
                .min_w(px(0.0))
                .max_w(px(92.0))
                .flex()
                .items_center()
                .px(px(8.0))
                .child(
                    text(market.symbol.clone())
                        .min_w(px(0.0))
                        .truncate()
                        .text_size(px(11.0))
                        .text_color(theme::TEXT),
                ),
        )
        .child(segment(market.kind.label().to_owned(), theme::TEXT_FAINT));
    if let Some(detail) = market.detail() {
        chooser = chooser.child(segment(detail, theme::WARNING));
    }
    let remove = view()
        .w(px(29.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::SURFACE)
        // A whole border of its own laid 1pt over the box's right edge, so
        // all four sides light up on hover and the line between them stays
        // one line.
        .relative()
        .ml(px(-1.0))
        .border_1()
        .border_color(theme::BORDER)
        .text_size(px(11.0))
        .text_color(theme::TEXT_MUTED)
        .role(AccessibilityRole::Button)
        .label(format!("Remove {} from Markets", market.symbol))
        .hover(|s| {
            s.bg(theme::PANEL)
                .text_color(theme::TEXT)
                .border_color(theme::ACCENT)
                .z_index(1)
        })
        .on_press(move || {
            let settings = saved.get_untracked();
            let markets = settings
                .markets()
                .into_iter()
                .filter(|kept| kept.identity() != identity)
                .collect();
            tab.save(&settings.with_markets(markets));
        })
        .child(text("×"));
    view().w_full().flex_row().child(chooser).child(remove)
}

/// "Max Trades": the old compact number field.
fn max_trades(tab: TabContext<TradeStreamSettings>) -> View {
    let saved = tab.settings;
    let field = number_field(
        move || saved.with(TradeStreamSettings::max_trades) as u16,
        MIN_TRADES..=MAX_TRADES,
        STEP,
        move |next| tab.save(&saved.get_untracked().with_max_trades(next)),
    )
    .label("Max Trades")
    .hint("10 to 200 trades")
    .test_id("trade-stream-max-trades");
    ui::form_row("Max Trades").child(field)
}
