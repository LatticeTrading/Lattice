//! The indicators' badge and the menu it opens.
//!
//! The badge floats on the plot beside its header, right of the exchange
//! it ends with: small, bordered and see-through until the pointer is
//! over it, as the badge that opens the drawing toolbar is. It is laid
//! over the plot's view, after it, so a press on it is its own. In a pane
//! too narrow for the header it stays inside the plot's area, over the
//! header's end.
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
    theme, ui,
    widgets::{
        TabContext,
        chart::ChartSettings,
        common::plot::{self, Plot},
    },
};

/// The badge: how wide and tall it is, how far right of the header's end,
/// and its icon's side.
const BADGE_WIDTH: f32 = 22.0;
const BADGE_HEIGHT: f32 = 16.0;
const BADGE_GAP: f32 = 8.0;
const BADGE_ICON: f32 = 12.0;
/// How opaque the badge is until the pointer is over it.
const BADGE_OPACITY: f32 = 0.6;
/// The menu's narrowest, and a row's height (a select's).
const MENU_WIDTH: f32 = 250.0;
const ROW_HEIGHT: f32 = 30.0;

/// What the badge and the menu are named.
const NAME: &str = "Indicators";

/// The badge that opens the menu of indicators for the chart in `tab`,
/// floating on `plot` (the chart's) beside its header, which says what
/// `title` does.
pub(crate) fn badge(tab: &TabContext<ChartSettings>, plot: &Plot, title: Memo<String>) -> View {
    let tab = tab.clone();
    let open = signal(false);
    let anchor = element_ref();
    let size = plot.size;
    let (area_left, _) = plot::AREA_ORIGIN;
    let (_, top) = plot::HEADER_ORIGIN;
    let menu = dynamic(move || open.get().then(|| menu(&tab, &anchor, open)));
    view()
        .element_ref(&anchor)
        .absolute()
        .left(move || {
            // Beside the header, on a whole point, and never past the
            // plot's area.
            let beside = (title.with(|title| plot::header_end(title)) + BADGE_GAP).ceil();
            let (width, _) = size.get();
            px(beside.min(area_left + width - BADGE_WIDTH).max(area_left))
        })
        .top(px(top))
        .w(px(BADGE_WIDTH))
        .h(px(BADGE_HEIGHT))
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::SURFACE.opacity(BADGE_OPACITY))
        .border_1()
        .border_color(theme::BORDER)
        .text_color(theme::TEXT_MUTED)
        .role(AccessibilityRole::Button)
        .label(NAME)
        .accessibility_state(move || AccessibilityState {
            expanded: Some(open.get()),
            has_popup: true,
            ..Default::default()
        })
        .test_id("chart-indicators")
        .tooltip(ui::hint(|| NAME.to_owned()))
        .hover(|s| {
            s.bg(theme::SURFACE)
                .border_color(theme::TEXT_FAINT)
                .text_color(theme::TEXT)
        })
        .active(|s| s.bg(theme::BORDER))
        .style_when(
            move || open.get(),
            |s| {
                s.bg(theme::SURFACE)
                    .border_color(theme::ACCENT)
                    .text_color(theme::ACCENT)
            },
        )
        .on_press(move || open.update(|open| *open = !*open))
        .child(icon(
            || include_str!("../../../../assets/icons/indicators.svg"),
            BADGE_ICON,
        ))
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
