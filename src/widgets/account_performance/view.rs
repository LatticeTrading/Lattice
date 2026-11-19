//! The Account Performance's view: the frame every account widget has
//! (what could not be read, the form while no account is set), without the
//! account's strip ([`bare_account_frame`]), around the plot, which is all
//! of it. The plot is the shared one ([`plot_view`]: the line on shader
//! tiles, the axes, the newest value's tag, the status, the crosshair and
//! the pointer's handling, all as the Chart's), with in its top left corner:
//!
//! - its header: the account's exchange's logo and its address (of several
//!   accounts: their exchanges' logos and how many they are), which opens
//!   the panel that changes them ([`account_badge`]), then the period's badge,
//!   which says the period plotted (`30D`) and opens the menu of them, as
//!   the Chart's timeframe badge does;
//! - under it, a line each: what the window plotted made (green above zero,
//!   red below) and traded.
//!
//! What is plotted (the account's value or its profit and loss) and of
//! which part of the account (all of it or its perpetuals) are the settings
//! panel's.

use std::rc::Rc;

use nsh_base::SelectItem;
use nsh_core::{
    overlay::{Align, Placement},
    prelude::*,
    Checked,
};

use super::{
    session::{AccountPerformanceSession, Readout},
    settings::{AccountPerformanceSettings, Choice},
};
use crate::{
    data::portfolio::Period,
    theme, ui,
    widgets::{
        common::{
            accounts::{account_badge, bare_account_frame, menu_badge, Tone},
            plot::{self, plot_view, PlotLook, Times},
        },
        TabContext,
    },
};

/// What its test ids start with.
const ID: &str = "account-performance";
/// How far apart the header's badges are.
const GAP: f32 = 6.0;
/// A figure's line under the header, its text's size, and how many there
/// are.
const LINE: f32 = 15.0;
const FIGURE_SIZE: f32 = 11.0;
const FIGURES: usize = 2;
/// The period menu's narrowest, and a row's height (the Chart's timeframe
/// menu's).
const MENU_WIDTH: f32 = 96.0;
const ROW_HEIGHT: f32 = 26.0;
/// What the period's badge and menu are named.
const PERIOD: &str = "Period";

/// The Account Performance of `session`, as its tab's settings say.
pub(super) fn performance_view(
    session: &Rc<AccountPerformanceSession>,
    tab: &TabContext<AccountPerformanceSettings>,
) -> View {
    bare_account_frame(
        tab,
        &session.portfolio,
        session.reports.standing,
        "account history",
        plot(session, tab),
    )
}

/// The compact select of a choice the settings hold, named `name` for
/// assistive tech: it shows what `read` reads of the settings, and a pick
/// saves what `write` makes of them. The settings panel's.
pub(super) fn choice_select<C: Choice>(
    tab: &TabContext<AccountPerformanceSettings>,
    name: &'static str,
    read: fn(&AccountPerformanceSettings) -> C,
    write: fn(AccountPerformanceSettings, C) -> AccountPerformanceSettings,
) -> impl IntoElement + use<C> {
    let (tab, saved) = (tab.clone(), tab.settings);
    ui::compact_dropdown(
        || {
            C::ALL
                .iter()
                .map(|choice| SelectItem::new(choice.id(), choice.label()))
                .collect()
        },
        move || SharedString::from(saved.with(|settings| read(settings).id())),
        move || saved.with(|settings| read(settings).label().to_owned()),
        move |id| {
            if let Some(choice) = C::from_id(id) {
                tab.save(&write(saved.get_untracked(), choice));
            }
        },
        name,
    )
}

/// The plot, with the figures over it and, laid over its view (after it,
/// so over its surfaces: a press on a badge is its own), the header.
fn plot(
    session: &Rc<AccountPerformanceSession>,
    tab: &TabContext<AccountPerformanceSettings>,
) -> View {
    let saved = tab.settings;
    let status = session.status;
    let look = PlotLook {
        id: ID,
        grid: computed(move || saved.with(AccountPerformanceSettings::grid)),
        // A point is one value, which the crosshair's tag on the axis
        // names.
        bar_prices: computed(|| false),
        status: computed(move || status.get()),
        pointer: None,
        legend: computed(|| FIGURES as f32 * LINE),
        // Samples are hours or days apart: a time of day alone says
        // nothing, and all time runs over years.
        times: computed(
            move || match saved.with(AccountPerformanceSettings::period) {
                Period::AllTime => Times::Date,
                _ => Times::DayTime,
            },
        ),
    };
    let over = view()
        .absolute()
        .size_full()
        .child(figures(session.readout));
    // The header stays inside the plot's area in a pane too narrow for it.
    let size = session.plot.size;
    let (area_left, _) = plot::AREA_ORIGIN;
    let (left, top) = plot::HEADER_ORIGIN;
    let header = view()
        .test_id(&format!("{ID}-header"))
        .absolute()
        .left(px(left))
        .top(px(top))
        .max_w(move || px((size.get().0 - (left - area_left)).max(0.0)))
        .flex_row()
        .items_center()
        .gap(px(GAP))
        .overflow_hidden()
        .child(account_badge(tab, &session.portfolio))
        .child(period_badge(tab));
    view()
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .relative()
        .flex_col()
        .child(plot_view(&session.plot, look, over))
        .child(header)
}

/// What the window plotted made and traded, a line each under the header.
fn figures(readout: Signal<Readout>) -> View {
    let (left, top) = plot::UNDER_HEADER;
    let made = figure(
        "PnL",
        move || readout.with(|readout| readout.pnl.clone()),
        move || Tone::color(readout.with(|readout| readout.tone)),
    )
    .test_id(&format!("{ID}-pnl"));
    let traded = figure(
        "Volume",
        move || readout.with(|readout| readout.volume.clone()),
        || theme::TEXT.get(),
    )
    .test_id(&format!("{ID}-volume"));
    view()
        .absolute()
        .left(px(left))
        .top(px(top))
        .flex_col()
        .text_size(px(FIGURE_SIZE))
        .child(made)
        .child(traded)
}

/// One figure: its name in faint text, then what it reads in `color`.
fn figure(
    name: &'static str,
    read: impl Fn() -> String + 'static,
    color: impl Fn() -> AdaptiveColor + 'static,
) -> View {
    view()
        .h(px(LINE))
        .flex_none()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .child(
            text(name)
                .flex_none()
                .number_of_lines(1)
                .text_color(theme::TEXT_FAINT),
        )
        .child(
            text(read)
                .flex_none()
                .number_of_lines(1)
                .tabular_nums()
                .font_weight(FontWeight::MEDIUM)
                .text_color(color),
        )
}

/// The badge that says the period plotted and opens the menu of them.
fn period_badge(tab: &TabContext<AccountPerformanceSettings>) -> View {
    let (saved, tab) = (tab.settings, tab.clone());
    let period = computed(move || saved.with(AccountPerformanceSettings::period));
    let open = signal(false);
    let anchor = element_ref();
    let menu = dynamic(move || open.get().then(|| period_menu(&tab, &anchor, open)));
    menu_badge(open)
        .element_ref(&anchor)
        .label(move || format!("{PERIOD} {}", period.get().label()))
        .test_id(&format!("{ID}-period"))
        .tooltip(ui::hint(|| PERIOD.to_owned()))
        .child(
            text(move || period.get().label().to_owned())
                .number_of_lines(1)
                .tabular_nums(),
        )
        .child(menu)
}

/// The period menu below `anchor` (its badge), which `open` says is open:
/// in an overlay, so it takes its presses wherever it hangs.
fn period_menu(
    tab: &TabContext<AccountPerformanceSettings>,
    anchor: &ElementRef,
    open: Signal<bool>,
) -> Overlay {
    let rows = Period::ALL.map(|period| period_row(tab.clone(), period, open));
    overlay()
        .anchor(anchor)
        .placement(Placement::Bottom)
        .align(Align::Start)
        .offset(px(2.0))
        .on_press_outside(move |_: &Point<Points>| open.set(false))
        .on_cancel(move || open.set(false))
        .child(
            view()
                .test_id(&format!("{ID}-period-menu"))
                .min_w(px(MENU_WIDTH))
                .flex_col()
                .py(px(4.0))
                .bg(theme::PANEL)
                .border_1()
                .border_color(theme::BORDER)
                .text_size(px(11.0))
                .text_color(theme::TEXT)
                .role(AccessibilityRole::Menu)
                .label(PERIOD)
                .children(rows),
        )
}

/// One period of the menu, in the accent while it is the one plotted. A
/// press plots it and closes the menu.
fn period_row(
    tab: TabContext<AccountPerformanceSettings>,
    period: Period,
    open: Signal<bool>,
) -> View {
    let saved = tab.settings;
    let plotted = move || saved.with(|settings| settings.period() == period);
    view()
        .h(px(ROW_HEIGHT))
        .w_full()
        .flex_row()
        .items_center()
        .px(px(10.0))
        .role(AccessibilityRole::Menuitem)
        .label(period.label())
        .accessibility_state(move || AccessibilityState {
            checked: Some(if plotted() {
                Checked::True
            } else {
                Checked::False
            }),
            ..Default::default()
        })
        .test_id(&format!("{ID}-period-{}", period.id()))
        .hover(|s| s.bg(theme::SURFACE))
        .active(|s| s.bg(theme::BORDER))
        .style_when(plotted, |s| s.text_color(theme::ACCENT))
        .on_press(move || {
            open.set(false);
            let settings = saved.get_untracked();
            if settings.period() != period {
                tab.save(&settings.with_period(period));
            }
        })
        .child(text(period.label()).number_of_lines(1).tabular_nums())
}
