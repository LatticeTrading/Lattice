//! The indicators' legend: a row an indicator under the plot's header, in
//! the order they were added, as TradingView lists a chart's under its
//! pair.
//!
//! A row says what the indicator is (`EMA 20`: its kind and its inputs)
//! and, in its color, the value of each of its lines at the bar under the
//! pointer, or at the newest bar while the pointer is off the plot (a
//! trend's line in the color of the side of the bars it is on there). With
//! the pointer over the row its controls show beside them: one hides the
//! indicator or shows it again (hidden, it stays in the legend, dimmed,
//! with that control), one opens its settings ([`super::panel`]) and one
//! removes it. Every change is saved in the chart's settings at once.
//!
//! The rows are laid over the plot's view, after it, so a press on one is
//! its own; the hovered bar's prices show under them ([`legend_height`]).

use std::rc::Rc;

use nsh_core::prelude::*;

use super::{Indicator, change, icon, panel};
use crate::{
    format::price_text,
    theme, ui,
    widgets::{
        TabContext,
        chart::{ChartSettings, session::ChartSession},
        common::plot,
    },
};

/// A row's height, and how far its text is from its edges.
const ROW: f32 = 18.0;
const PAD: f32 = 4.0;
/// A control's side, and its icon's.
const CONTROL: f32 = 16.0;
const ICON: f32 = 12.0;
/// What a line with no value at the bar says.
const NO_VALUE: &str = "--";

/// How many points under the plot's header the rows of `count` indicators
/// take.
pub(crate) fn legend_height(count: usize) -> f32 {
    count as f32 * ROW
}

/// The legend of the chart of `session` in `tab`: a row an indicator of
/// its settings, under the plot's header and kept to the plot's area (a
/// pane too small for them all cuts them short).
pub(crate) fn legend(session: &Rc<ChartSession>, tab: &TabContext<ChartSettings>) -> View {
    let saved = tab.settings;
    let count = computed(move || saved.with(|settings| settings.indicators().len()));
    let size = session.plot.size;
    let (left, top) = plot::UNDER_HEADER;
    let (area_left, area_top) = plot::AREA_ORIGIN;
    let (session, tab) = (session.clone(), tab.clone());
    view()
        .absolute()
        .left(px(left - PAD))
        .top(px(top))
        .max_w(move || px((size.get().0 - (left - PAD - area_left)).max(0.0)))
        .max_h(move || px((size.get().1 - (top - area_top)).max(0.0)))
        .flex_col()
        .items_start()
        .overflow_hidden()
        .test_id("chart-legend")
        .child(each(
            move || (0..count.get()).collect::<Vec<_>>(),
            |at: &usize| *at,
            move |at, _| row(&session, &tab, at.get_untracked()),
        ))
}

/// The row of the settings' indicator `at`.
fn row(session: &Rc<ChartSession>, tab: &TabContext<ChartSettings>, at: usize) -> View {
    let saved = tab.settings;
    let shown = computed(move || saved.with(|settings| settings.indicators().get(at).cloned()));
    let title = computed(move || {
        shown.with(|shown| shown.as_ref().map_or_else(String::new, Indicator::title))
    });
    let hidden =
        computed(move || shown.with(|shown| shown.as_ref().is_some_and(Indicator::hidden)));
    let (lines, scene, pointer) = (session.indicators, session.plot.scene, session.plot.pointer);
    // Its lines' values at the bar under the pointer, else at the newest,
    // and the side of the bars a trend's line is on there; nothing while
    // it is hidden or not computed yet.
    let values = computed(move || {
        let lines = lines.get();
        let said = shown.with(|shown| {
            let lines_of = lines.0.lines_of(at, shown.as_ref()?)?;
            let under = pointer
                .get()
                .and_then(|(x, _)| scene.with(|scene| scene.0.bar_near(x)));
            let bar = under.or_else(|| lines.0.bars().checked_sub(1))?;
            let values = lines_of
                .iter()
                .map(|line| line.at(bar).map_or_else(|| NO_VALUE.to_owned(), price_text));
            let rising = lines_of.first().and_then(|line| line.rising_at(bar));
            Some((values.collect::<Vec<_>>().join("  "), rising))
        });
        said.unwrap_or_default()
    });
    // Its settings panel is open.
    let open = signal(false);
    let gear = element_ref();
    let visible = tab.shown;
    let settings = {
        let tab = tab.clone();
        dynamic(move || {
            (open.get() && visible.get())
                .then(|| panel::panel(&tab, at, &gear, move || open.set(false)))
                .flatten()
        })
    };

    let (hide, remove) = (tab.clone(), tab.clone());
    let hides = control(
        move || {
            if hidden.get() {
                include_str!("../../../../assets/icons/eye-off.svg")
            } else {
                include_str!("../../../../assets/icons/eye.svg")
            }
        },
        move || {
            let verb = if hidden.get() { "Show" } else { "Hide" };
            format!("{verb} {}", title.get())
        },
    )
    // Hidden, it is what brings the indicator back: it stays.
    .style_when(move || hidden.get(), |s| s.opacity(1.0))
    .test_id(&format!("chart-indicator-{at}-hide"))
    .on_press(move || {
        change(&hide, at, |indicator| {
            let hidden = indicator.hidden();
            Some(indicator.with_hidden(!hidden))
        });
    });
    let sets = control(
        || include_str!("../../../../assets/icons/settings.svg"),
        move || format!("{} settings", title.get()),
    )
    .element_ref(&gear)
    .style_when(
        move || open.get(),
        |s| s.opacity(1.0).text_color(theme::ACCENT),
    )
    .accessibility_state(move || AccessibilityState {
        expanded: Some(open.get()),
        has_popup: true,
        ..Default::default()
    })
    .test_id(&format!("chart-indicator-{at}-settings"))
    .on_press(move || open.update(|open| *open = !*open));
    let removes = control(
        || include_str!("../../../../assets/icons/remove.svg"),
        move || format!("Remove {}", title.get()),
    )
    .hover(|s| s.bg(theme::BORDER).text_color(theme::DANGER))
    .test_id(&format!("chart-indicator-{at}-remove"))
    .on_press(move || {
        open.set(false);
        change(&remove, at, |_| None);
    });

    view()
        .h(px(ROW))
        .flex_none()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .px(px(PAD))
        .text_size(px(11.0))
        .group()
        .hover(|s| s.bg(theme::SURFACE.opacity(0.85)))
        .test_id(&format!("chart-indicator-{at}"))
        .child(
            text(move || title.get())
                .flex_none()
                .number_of_lines(1)
                .text_color(move || {
                    if hidden.get() {
                        theme::TEXT_FAINT.get()
                    } else {
                        theme::TEXT_MUTED.get()
                    }
                }),
        )
        .child(
            text(move || values.with(|(said, _)| said.clone()))
                .flex_none()
                .number_of_lines(1)
                .text_color(move || match values.with(|(_, rising)| *rising) {
                    Some(true) => theme::BID.get(),
                    Some(false) => theme::ASK.get(),
                    None => {
                        let color = shown.with(|shown| shown.as_ref().map(Indicator::color));
                        color.map_or_else(|| theme::TEXT_MUTED.get(), |color| color.role().get())
                    }
                })
                .style_when(
                    move || values.with(|(said, _)| said.is_empty()),
                    |s| s.hidden(),
                )
                .test_id(&format!("chart-indicator-{at}-values")),
        )
        .child(
            view()
                .flex_none()
                .flex_row()
                .items_center()
                .child(hides)
                .child(sets)
                .child(removes),
        )
        .child(settings)
}

/// A control of a row: the icon `markup` gives, named `name`, which shows
/// while the pointer is over its row.
fn control(
    markup: impl Fn() -> &'static str + 'static,
    name: impl Fn() -> String + Clone + 'static,
) -> View {
    let hint = name.clone();
    view()
        .size(px(CONTROL))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .text_color(theme::TEXT_MUTED)
        .opacity(0.0_f32)
        .group_hover(|s| s.opacity(1.0))
        .role(AccessibilityRole::Button)
        .label(name)
        .tooltip(ui::hint(hint))
        .hover(|s| s.bg(theme::BORDER).text_color(theme::TEXT))
        .active(|s| s.bg(theme::TEXT_FAINT.opacity(0.4)))
        .child(icon(markup, ICON))
}
