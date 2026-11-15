//! The indicators' badge and the menu it opens.
//!
//! The badge is the second beside the plot's header ([`badges`]), right
//! of the timeframe's: its icon and the word `INDICATORS`.
//!
//! A press opens the menu below it: a row a kind of indicator
//! ([`Kind::ALL`]), its whole name and its short one. Pressing a row adds
//! one of that kind to the chart as it starts ([`Indicator::new`]) and
//! closes the menu, as Escape and a press outside it do.

use nsh_core::{
    overlay::{Align, Placement},
    prelude::*,
};

use super::{Indicator, Kind, icon};
use crate::{
    theme,
    widgets::{
        TabContext,
        chart::{ChartSettings, badges},
    },
};

/// What the badge says, its icon's side and how far the icon is from the
/// word.
const WORD: &str = "INDICATORS";
const BADGE_ICON: f32 = 12.0;
const BADGE_SPACE: f32 = 4.0;
/// How wide the badge is.
pub(crate) const BADGE_WIDTH: f32 = badges::width(WORD.len(), BADGE_ICON + BADGE_SPACE);
/// The menu's narrowest, and a row's height (a select's).
const MENU_WIDTH: f32 = 250.0;
const ROW_HEIGHT: f32 = 30.0;

/// What the badge and the menu are named.
const NAME: &str = "Indicators";

/// The badge that opens the menu of indicators for the chart in `tab`,
/// `left` points from the plot's view's left edge.
pub(crate) fn badge(tab: &TabContext<ChartSettings>, left: impl Fn() -> f32 + 'static) -> View {
    let tab = tab.clone();
    let open = signal(false);
    let anchor = element_ref();
    let menu = dynamic(move || open.get().then(|| menu(&tab, &anchor, open)));
    // The icon and the word in a row of their own: the menu is the
    // badge's child too, and is not spaced from them.
    let said = view()
        .flex_row()
        .items_center()
        .gap(px(BADGE_SPACE))
        .child(icon(
            || include_str!("../../../../assets/icons/indicators.svg"),
            BADGE_ICON,
        ))
        .child(text(WORD).number_of_lines(1));
    badges::floating(left, || BADGE_WIDTH, open)
        .element_ref(&anchor)
        .label(NAME)
        .test_id("chart-indicators")
        .child(said)
        .child(menu)
}

/// The menu below `anchor` (the badge), which `open` says is open: in an
/// overlay, so it takes its presses wherever it hangs.
fn menu(tab: &TabContext<ChartSettings>, anchor: &ElementRef, open: Signal<bool>) -> Overlay {
    let rows = Kind::ALL.map(|kind| row(tab.clone(), kind, open));
    overlay()
        .anchor(anchor)
        .placement(Placement::Bottom)
        .align(Align::Start)
        .offset(px(2.0))
        .on_press_outside(move |_: &Point<Points>| open.set(false))
        .on_cancel(move || open.set(false))
        .child(
            view()
                .test_id("chart-indicator-menu")
                .min_w(px(MENU_WIDTH))
                .flex_col()
                .py(px(4.0))
                .bg(theme::PANEL)
                .border_1()
                .border_color(theme::BORDER)
                .text_size(px(11.0))
                .text_color(theme::TEXT)
                .role(AccessibilityRole::Menu)
                .label(NAME)
                .children(rows),
        )
}

/// One kind of the menu: its whole name, and its short one at the row's
/// far edge. A press adds one to the chart and closes the menu.
fn row(tab: TabContext<ChartSettings>, kind: Kind, open: Signal<bool>) -> View {
    view()
        .h(px(ROW_HEIGHT))
        .w_full()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .px(px(10.0))
        .role(AccessibilityRole::Menuitem)
        .label(format!("Add {}", kind.name()))
        .test_id(&format!("chart-indicator-add-{}", kind.id()))
        .hover(|s| s.bg(theme::SURFACE))
        .active(|s| s.bg(theme::BORDER))
        .on_press(move || {
            open.set(false);
            let settings = tab.settings.get_untracked();
            let mut indicators = settings.indicators().to_vec();
            indicators.push(Indicator::new(kind, &indicators));
            tab.save(&settings.with_indicators(indicators));
        })
        .child(text(kind.name()).flex_1().min_w(px(0.0)).number_of_lines(1))
        .child(
            text(kind.label())
                .flex_none()
                .text_size(px(10.0))
                .text_color(theme::TEXT_FAINT),
        )
}
