//! The timeframe's badge and the menu it opens.
//!
//! The badge is the first beside the plot's header ([`badges`]) and says
//! the interval charted (`1m`, `Tick`), which the header leaves to it.
//!
//! A press opens the menu below it: a row an interval ([`Interval::ALL`]),
//! the one charted in the accent. Pressing a row charts that interval, as
//! the settings panel's Timeframe select does, and closes the menu, as
//! Escape and a press outside it do.

use nsh_core::{
    Checked,
    overlay::{Align, Placement},
    prelude::*,
};

use super::{ChartSettings, badges, settings::Interval};
use crate::{theme, ui, widgets::TabContext};

/// The menu's narrowest, and a row's height.
const MENU_WIDTH: f32 = 96.0;
const ROW_HEIGHT: f32 = 26.0;

/// What the badge and the menu are named.
const NAME: &str = "Timeframe";

/// How wide the badge is while it says `interval`.
pub(super) fn badge_width(interval: Interval) -> f32 {
    badges::width(interval.label().len(), 0.0)
}

/// The badge that says the interval the chart in `tab` charts and opens
/// the menu of them, `left` points from the plot's view's left edge.
pub(super) fn badge(tab: &TabContext<ChartSettings>, left: impl Fn() -> f32 + 'static) -> View {
    let (saved, tab) = (tab.settings, tab.clone());
    let interval = computed(move || saved.with(ChartSettings::interval));
    let open = signal(false);
    let anchor = element_ref();
    let menu = dynamic(move || open.get().then(|| menu(&tab, &anchor, open)));
    badges::floating(left, move || badge_width(interval.get()), open)
        .element_ref(&anchor)
        .label(move || format!("{NAME} {}", interval.get().label()))
        .test_id("chart-timeframe")
        .tooltip(ui::hint(|| NAME.to_owned()))
        .child(text(move || interval.get().label().to_owned()).number_of_lines(1))
        .child(menu)
}

/// The menu below `anchor` (the badge), which `open` says is open: in an
/// overlay, so it takes its presses wherever it hangs.
fn menu(tab: &TabContext<ChartSettings>, anchor: &ElementRef, open: Signal<bool>) -> Overlay {
    let rows = Interval::ALL.map(|interval| row(tab.clone(), interval, open));
    overlay()
        .anchor(anchor)
        .placement(Placement::Bottom)
        .align(Align::Start)
        .offset(px(2.0))
        .on_press_outside(move |_: &Point<Points>| open.set(false))
        .on_cancel(move || open.set(false))
        .child(
            view()
                .test_id("chart-timeframe-menu")
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

/// One interval of the menu: its name, in the accent while it is the one
/// charted. A press charts it and closes the menu.
fn row(tab: TabContext<ChartSettings>, interval: Interval, open: Signal<bool>) -> View {
    let saved = tab.settings;
    let charted = move || saved.with(|settings| settings.interval() == interval);
    view()
        .h(px(ROW_HEIGHT))
        .w_full()
        .flex_row()
        .items_center()
        .px(px(10.0))
        .role(AccessibilityRole::Menuitem)
        .label(interval.label())
        .accessibility_state(move || AccessibilityState {
            checked: Some(if charted() {
                Checked::True
            } else {
                Checked::False
            }),
            ..Default::default()
        })
        .test_id(&format!("chart-timeframe-{}", interval.id()))
        .hover(|s| s.bg(theme::SURFACE))
        .active(|s| s.bg(theme::BORDER))
        .style_when(charted, |s| s.text_color(theme::ACCENT))
        .on_press(move || {
            open.set(false);
            let settings = saved.get_untracked();
            if settings.interval() != interval {
                tab.save(&settings.with_interval(interval));
            }
        })
        .child(text(interval.label()).number_of_lines(1))
}
