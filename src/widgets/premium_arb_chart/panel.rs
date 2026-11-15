//! The Premium/Arb Chart's settings panel, laid out as the old app's
//! `SettingsPanel` drew its schema (`premiumArbChartSettingsSchema`): a row
//! per setting, its name on the left and its control on the right, as the
//! Chart's. The pane's gear opens it for the active tab; every change is
//! saved in the tab record at once.
//!
//! - Primary Pair, Pair 2: each market in the old chooser box (logo, pair,
//!   type, detail), `Select...` while it has none; pressing it opens the
//!   market picker ([`picker`], which the widget's own chooser opens too).
//! - Display Mode: the old compact select (Raw Delta, % Delta).
//! - Show Grid: the old square checkbox.

use nsh_base::SelectItem;
use nsh_core::prelude::*;

use super::{PremiumArbSettings, settings::Display};
use crate::{
    data::ferris::{Channel, Operation, Requirement},
    ui::{self, Side},
    widgets::{
        TabContext,
        common::{MarketPicker, books::Pair, empty_market_chooser, market_chooser, market_name},
    },
};

/// The old panel's width.
const PANEL_WIDTH: Points = px(340.0);
/// A market box's widest (the old `.settings-symbol-search-btn`).
const CHOOSER_WIDTH: f32 = 260.0;

/// What the widget calls a pair's market (the old schema's labels).
pub(super) const fn pair_name(pair: Pair) -> &'static str {
    match pair {
        Pair::Primary => "Primary Pair",
        Pair::Secondary => "Pair 2",
    }
}

/// The settings panel of the chart's tab, below the pane's gear.
pub(super) fn panel(tab: TabContext<PremiumArbSettings>) -> Overlay {
    let saved = tab.settings;
    // The pair the market picker is open for.
    let picking = signal(None::<Pair>);
    let grid = {
        let tab = tab.clone();
        ui::check_box(
            "Show Grid",
            move || saved.with(PremiumArbSettings::grid),
            move || {
                let settings = saved.get_untracked();
                let shown = settings.grid();
                tab.save(&settings.with_grid(!shown));
            },
        )
    };
    let body = view()
        .test_id("premium-arb-chart-settings")
        .flex_col()
        .py(px(4.0))
        .child(market_row(saved, Pair::Primary, picking))
        .child(market_row(saved, Pair::Secondary, picking))
        .child(display_row(tab.clone()))
        .child(ui::label_edge_row("Show Grid").child(grid))
        .child(picker(&tab, picking));
    let close = {
        let tab = tab.clone();
        move || tab.close_settings()
    };
    ui::settings_panel(
        tab.settings_anchor(),
        Side::Right,
        PANEL_WIDTH,
        "Premium/Arb Chart settings",
        close,
        body,
    )
}

/// "Primary Pair" or "Pair 2": the pair's market in its box, which opens
/// the market picker (by setting `picking`).
fn market_row(saved: Memo<PremiumArbSettings>, pair: Pair, picking: Signal<Option<Pair>>) -> View {
    let market = computed(move || saved.with(|settings| settings.market(pair)));
    let chooser = dynamic(move || {
        let market = market.get();
        let (chooser, name) = match &market {
            Some(market) => (market_chooser(market), market_name(market)),
            None => (empty_market_chooser(), "Select...".to_owned()),
        };
        chooser
            .flex_shrink(1.0)
            .min_w(px(0.0))
            .max_w(px(CHOOSER_WIDTH))
            .role(AccessibilityRole::Button)
            .label(format!("{} {name}", pair_name(pair)))
            .accessibility_state(move || AccessibilityState {
                expanded: Some(picking.get() == Some(pair)),
                has_popup: true,
                ..Default::default()
            })
            .test_id(&format!("premium-arb-chart-market-{}", pair.id()))
            .on_press(move || picking.set(Some(pair)))
    });
    ui::label_edge_row(pair_name(pair)).child(chooser)
}

/// "Display Mode": the raw difference, or its percentage of the primary's
/// price.
fn display_row(tab: TabContext<PremiumArbSettings>) -> View {
    let saved = tab.settings;
    let select = ui::compact_dropdown(
        || {
            Display::ALL
                .iter()
                .map(|display| SelectItem::new(display.id(), display.label()))
                .collect()
        },
        move || SharedString::from(saved.with(|settings| settings.display().id())),
        move || saved.with(|settings| settings.display().label().to_owned()),
        move |id| {
            if let Some(display) = Display::from_id(id) {
                tab.save(&saved.get_untracked().with_display(display));
            }
        },
        "Display Mode",
    );
    ui::label_edge_row("Display Mode").child(select)
}

/// The market picker of the pair `picking` names, while it names one: a
/// pick or Clear is saved in `tab`'s settings, and either lets go of the
/// pair.
pub(super) fn picker(
    tab: &TabContext<PremiumArbSettings>,
    picking: Signal<Option<Pair>>,
) -> impl IntoElement + use<> {
    let tab = tab.clone();
    dynamic(move || {
        picking
            .get()
            .map(|pair| market_picker(&tab, pair, move || picking.set(None)))
    })
}

/// The market picker for `pair`'s market, offering the markets whose
/// exchange serves a book both ways (a snapshot and a live topic), as the
/// old one did: the chart's prices are read from books.
fn market_picker(
    tab: &TabContext<PremiumArbSettings>,
    pair: Pair,
    on_close: impl Fn() + Clone + 'static,
) -> View {
    let saved = tab.settings;
    let current = saved.get_untracked().market(pair);
    let save = {
        let tab = tab.clone();
        move |market| tab.save(&saved.get_untracked().with_market(pair, market))
    };
    let pick = {
        let save = save.clone();
        move |picked| save(Some(picked))
    };
    let mut picker = MarketPicker::new(pick).requires(Requirement {
        snapshot: Some(Operation::FetchOrderBook),
        realtime: Some(Channel::OrderBook),
    });
    if current.is_some() {
        picker = picker.on_clear(&format!("Clear {}", pair_name(pair)), move || save(None));
    }
    picker.current(current).view(on_close)
}
