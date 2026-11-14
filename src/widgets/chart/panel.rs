//! The chart's settings panel, laid out as the old app's `SettingsPanel`
//! drew the chart's schema (`SettingsPanel.css`, `chartSettingsSchema`): a
//! row per setting, its name on the left and its control on the right, as
//! the Trade Stream's. The pane's gear opens it for the active tab; every
//! change is saved in the tab record at once.
//!
//! - Timeframe: the old compact select (ticks, or candles of one of the
//!   seven timeframes).
//! - Market: the market charted, in the old chooser box (logo, pair, type,
//!   detail) the Trade Stream's markets are in; pressing it opens the
//!   market picker, which offers the markets whose exchange can serve the
//!   timeframe: live trades for a tick chart, candle history and live
//!   candles otherwise. With a market picked, the picker's Clear goes back
//!   to the default one.
//! - Show Grid, Show OHLC on Hover: the old square checkboxes.

use nsh_base::SelectItem;
use nsh_core::prelude::*;

use super::{ChartSettings, settings::Interval};
use crate::{
    data::ferris::{Channel, Operation, Requirement},
    ui::{self, Side},
    widgets::{
        TabContext,
        common::{MarketPicker, market_chooser, market_name},
    },
};

/// The old panel's width.
const PANEL_WIDTH: Points = px(340.0);
/// The market box's widest (the old `.settings-symbol-search-btn`).
const CHOOSER_WIDTH: f32 = 260.0;

/// The settings panel of the chart's tab, below the pane's gear.
pub(super) fn panel(tab: TabContext<ChartSettings>) -> Overlay {
    let saved = tab.settings;
    let picking = signal(false);
    let picker = {
        let tab = tab.clone();
        dynamic(move || picking.get().then(|| picker(&tab, picking)))
    };
    let toggle = |name: &'static str,
                  shown: fn(&ChartSettings) -> bool,
                  set: fn(ChartSettings, bool) -> ChartSettings| {
        let tab = tab.clone();
        let check = ui::check_box(
            name,
            move || saved.with(shown),
            move || {
                let settings = saved.get_untracked();
                let next = !shown(&settings);
                tab.save(&set(settings, next));
            },
        );
        ui::label_edge_row(name).child(check)
    };
    let body = view()
        .test_id("chart-settings")
        .flex_col()
        .py(px(4.0))
        .child(timeframe_row(tab.clone()))
        .child(market_row(saved, picking))
        .child(toggle(
            "Show Grid",
            ChartSettings::grid,
            ChartSettings::with_grid,
        ))
        .child(toggle(
            "Show OHLC on Hover",
            ChartSettings::ohlc,
            ChartSettings::with_ohlc,
        ))
        .child(picker);
    let close = {
        let tab = tab.clone();
        move || tab.close_settings()
    };
    ui::settings_panel(
        tab.settings_anchor(),
        Side::Right,
        PANEL_WIDTH,
        "Chart settings",
        close,
        body,
    )
}

/// "Timeframe": ticks, or candles of one of the seven timeframes.
fn timeframe_row(tab: TabContext<ChartSettings>) -> View {
    let saved = tab.settings;
    let select = ui::compact_dropdown(
        || {
            Interval::ALL
                .iter()
                .map(|interval| SelectItem::new(interval.id(), interval.label()))
                .collect()
        },
        move || SharedString::from(saved.with(|settings| settings.interval().id())),
        move || saved.with(|settings| settings.interval().label().to_owned()),
        move |id| {
            if let Some(interval) = Interval::ALL
                .into_iter()
                .find(|interval| interval.id() == id)
            {
                tab.save(&saved.get_untracked().with_interval(interval));
            }
        },
        "Timeframe",
    );
    ui::label_edge_row("Timeframe").child(select)
}

/// "Market": the market charted, in its box, which opens the market
/// picker.
fn market_row(saved: Memo<ChartSettings>, picking: Signal<bool>) -> View {
    let market = computed(move || saved.with(ChartSettings::market));
    let chooser = dynamic(move || {
        let market = market.get();
        market_chooser(&market)
            .flex_shrink(1.0)
            .min_w(px(0.0))
            .max_w(px(CHOOSER_WIDTH))
            .role(AccessibilityRole::Button)
            .label(format!("Market {}", market_name(&market)))
            .accessibility_state(move || AccessibilityState {
                expanded: Some(picking.get()),
                has_popup: true,
                ..Default::default()
            })
            .test_id("chart-market")
            .on_press(move || picking.set(true))
    });
    ui::label_edge_row("Market").child(chooser)
}

/// The market picker; a pick (or Clear) is saved and closes it.
fn picker(tab: &TabContext<ChartSettings>, picking: Signal<bool>) -> View {
    let saved = tab.settings;
    let settings = saved.get_untracked();
    let requirement = match settings.interval() {
        // A tick chart never asks for trade history.
        Interval::Tick => Requirement::realtime(Channel::Trades),
        Interval::Candles(_) => Requirement {
            snapshot: Some(Operation::FetchOhlcv),
            realtime: Some(Channel::Ohlcv),
        },
    };
    let pick = {
        let tab = tab.clone();
        move |picked| tab.save(&saved.get_untracked().with_market(Some(picked)))
    };
    let mut picker = MarketPicker::new(pick)
        .requires(requirement)
        .current(Some(settings.market()));
    if settings.has_market() {
        let tab = tab.clone();
        picker = picker.on_clear("Chart the default market", move || {
            tab.save(&saved.get_untracked().with_market(None));
        });
    }
    picker.view(move || picking.set(false))
}
