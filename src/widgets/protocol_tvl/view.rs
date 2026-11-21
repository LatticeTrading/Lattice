//! The Protocol TVL's view, a column of full-width blocks:
//!
//! - what is wrong with its data, as a warning strip, hidden while nothing
//!   is: DefiLlama not answering the backend, a refresh that failed (the
//!   last data stays plotted), or nothing to plot at all;
//! - the plot, which is the shared one ([`plot_view`]: the line on shader
//!   tiles, the axes in short dollar figures, the newest value's tag, the
//!   status, the crosshair and the pointer's handling, all as the Chart's),
//!   with in its top left corner its header: the protocol's badge, which
//!   opens the picker ([`super::picker`]), then the range's badge, which
//!   says the range plotted (`1Y`) and opens the menu of them, as the
//!   Chart's timeframe badge does; and under it, a line each: the latest
//!   figure and how far it moved over the range (green up, red down);
//! - a strip: the chains the value is on, each with its share (all of
//!   them, in dollars, in its tooltip), then the credit DefiLlama asks for,
//!   which opens the protocol's page there.

use std::rc::Rc;

use nsh_core::{
    Checked, LineHeight,
    overlay::{Align, Placement},
    prelude::*,
};

use super::{
    picker,
    session::{self, ProtocolTvlSession, Readout, Sign, Standing},
    settings::{ProtocolTvlSettings, Range},
};
use crate::{
    data::ferris::defillama::SOURCE_URL,
    format, theme, ui,
    widgets::{
        TabContext,
        common::{
            accounts::menu_badge,
            plot::{self, PlotLook, Times, plot_view},
            source_credit,
        },
    },
};

/// What its test ids start with.
const ID: &str = "protocol-tvl";
/// How far apart the header's badges are.
const GAP: f32 = 6.0;
/// A figure's line under the header, its text's size, and how many there
/// are.
const LINE: f32 = 15.0;
const FIGURE_SIZE: f32 = 11.0;
const FIGURES: usize = 2;
/// The range menu's narrowest, and a row's height (the Chart's timeframe
/// menu's).
const MENU_WIDTH: f32 = 96.0;
const ROW_HEIGHT: f32 = 26.0;
/// The strip under the plot: 28 and its line.
const STRIP_HEIGHT: f32 = 29.0;
/// What the range's badge and menu are named.
const RANGE: &str = "Range";

/// The Protocol TVL of `session`, as its tab's settings say.
pub(super) fn tvl_view(
    session: &Rc<ProtocolTvlSession>,
    tab: &TabContext<ProtocolTvlSettings>,
) -> View {
    let standing = session.standing;
    // A tab that is not shown has no picker open.
    let (shown, open) = (tab.shown, session.search.open);
    effect(move || {
        if !shown.get() && open.get_untracked() {
            open.set(false);
        }
    });
    view()
        .test_id(ID)
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .flex_col()
        .overflow_hidden()
        .bg(theme::BACKGROUND)
        .text_color(theme::TEXT)
        .child(warning(move || standing.with(Standing::warning)))
        .child(plot(session, tab))
        .child(strip(session))
}

/// A strip over the plot warning of `content`, with a line under it: the
/// warning color, on a tint of it, behind a thick left edge. Hidden while
/// there is nothing to say.
fn warning(content: impl Fn() -> Option<String> + 'static) -> View {
    let content = computed(content);
    view()
        .flex_none()
        .w_full()
        .px(px(10.0))
        .py(px(5.0))
        .border_b_1()
        .border_color(theme::BORDER)
        .border_l(px(3.0))
        .border_l_color(theme::WARNING)
        .bg(theme::WARNING.opacity(0.1))
        .text_size(px(11.0))
        .text_color(theme::WARNING)
        .role(AccessibilityRole::Alert)
        .test_id(&format!("{ID}-warning"))
        .style_when(move || content.with(Option::is_none), |s| s.hidden())
        .child(
            text(move || content.get().unwrap_or_default()).line_height(LineHeight::Multiple(1.5)),
        )
}

/// The plot, with the figures over it and, laid over its view (after it,
/// so over its surfaces: a press on a badge is its own), the header.
fn plot(session: &Rc<ProtocolTvlSession>, tab: &TabContext<ProtocolTvlSettings>) -> View {
    let saved = tab.settings;
    let status = session.status;
    let look = PlotLook {
        id: ID,
        grid: computed(move || saved.with(ProtocolTvlSettings::grid)),
        // A point is one value, which the crosshair's tag on the axis
        // names.
        bar_prices: computed(|| false),
        status: computed(move || status.get()),
        pointer: None,
        legend: computed(|| FIGURES as f32 * LINE),
        // A point is a day: a time of day says nothing.
        times: computed(|| Times::Date),
        values: format::compact_usd,
    };
    let range = computed(move || saved.with(ProtocolTvlSettings::range));
    let over = view()
        .absolute()
        .size_full()
        .child(figures(session.readout, range));
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
        .child(protocol_badge(session, tab))
        .child(range_badge(tab));
    view()
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .relative()
        .flex_col()
        .child(plot_view(&session.plot, look, over))
        .child(header)
}

/// The latest figure and how far it moved over the range, a line each
/// under the header.
fn figures(readout: Signal<Readout>, range: Memo<Range>) -> View {
    let (left, top) = plot::UNDER_HEADER;
    let latest = figure(
        move || readout.with(|readout| readout.measure.to_owned()),
        move || readout.with(|readout| readout.value.clone()),
        || theme::TEXT.get(),
    )
    .test_id(&format!("{ID}-value"));
    let moved = figure(
        move || range.get().label().to_owned(),
        move || readout.with(|readout| readout.change.clone()),
        move || match readout.with(|readout| readout.sign) {
            Sign::Up => theme::POSITIVE.get(),
            Sign::Down => theme::NEGATIVE.get(),
            Sign::Flat => theme::TEXT_MUTED.get(),
        },
    )
    .test_id(&format!("{ID}-change"));
    view()
        .absolute()
        .left(px(left))
        .top(px(top))
        .flex_col()
        .text_size(px(FIGURE_SIZE))
        .child(latest)
        .child(moved)
}

/// One figure: its name in faint text, then what it reads in `color`.
fn figure(
    name: impl Fn() -> String + 'static,
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

/// The badge that names the protocol charted and opens the picker of them:
/// the name DefiLlama gives it once it has answered, its slug's words
/// until then.
fn protocol_badge(session: &Rc<ProtocolTvlSession>, tab: &TabContext<ProtocolTvlSettings>) -> View {
    let (saved, tvl) = (tab.settings, session.tvl);
    let name = computed(move || {
        tvl.with(|tvl| tvl.as_ref().map(|tvl| tvl.0.protocol.name.clone()))
            .unwrap_or_else(|| saved.with(|settings| session::titled(settings.protocol())))
    });
    let open = session.search.open;
    let anchor = element_ref();
    let panel = {
        let (session, tab) = (session.clone(), tab.clone());
        dynamic(move || open.get().then(|| picker::panel(&session, &tab, &anchor)))
    };
    menu_badge(open)
        .element_ref(&anchor)
        .label(move || format!("Change protocol: {}", name.get()))
        .test_id(&format!("{ID}-protocol"))
        .tooltip(ui::hint(|| "Protocol".to_owned()))
        .child(text(move || name.get()).number_of_lines(1))
        .child(panel)
}

/// The badge that says the range plotted and opens the menu of them.
fn range_badge(tab: &TabContext<ProtocolTvlSettings>) -> View {
    let (saved, tab) = (tab.settings, tab.clone());
    let range = computed(move || saved.with(ProtocolTvlSettings::range));
    let open = signal(false);
    let anchor = element_ref();
    let menu = dynamic(move || open.get().then(|| range_menu(&tab, &anchor, open)));
    menu_badge(open)
        .element_ref(&anchor)
        .label(move || format!("{RANGE} {}", range.get().label()))
        .test_id(&format!("{ID}-range"))
        .tooltip(ui::hint(|| RANGE.to_owned()))
        .child(
            text(move || range.get().label().to_owned())
                .number_of_lines(1)
                .tabular_nums(),
        )
        .child(menu)
}

/// The range menu below `anchor` (its badge), which `open` says is open:
/// in an overlay, so it takes its presses wherever it hangs.
fn range_menu(
    tab: &TabContext<ProtocolTvlSettings>,
    anchor: &ElementRef,
    open: Signal<bool>,
) -> Overlay {
    let rows = Range::ALL.map(|range| range_row(tab.clone(), range, open));
    overlay()
        .anchor(anchor)
        .placement(Placement::Bottom)
        .align(Align::Start)
        .offset(px(2.0))
        .on_press_outside(move |_: &Point<Points>| open.set(false))
        .on_cancel(move || open.set(false))
        .child(
            view()
                .test_id(&format!("{ID}-range-menu"))
                .min_w(px(MENU_WIDTH))
                .flex_col()
                .py(px(4.0))
                .bg(theme::PANEL)
                .border_1()
                .border_color(theme::BORDER)
                .text_size(px(11.0))
                .text_color(theme::TEXT)
                .role(AccessibilityRole::Menu)
                .label(RANGE)
                .children(rows),
        )
}

/// One range of the menu, in the accent while it is the one plotted. A
/// press plots it and closes the menu.
fn range_row(tab: TabContext<ProtocolTvlSettings>, range: Range, open: Signal<bool>) -> View {
    let saved = tab.settings;
    let plotted = move || saved.with(|settings| settings.range() == range);
    view()
        .h(px(ROW_HEIGHT))
        .w_full()
        .flex_row()
        .items_center()
        .px(px(10.0))
        .role(AccessibilityRole::Menuitem)
        .label(range.label())
        .accessibility_state(move || AccessibilityState {
            checked: Some(if plotted() {
                Checked::True
            } else {
                Checked::False
            }),
            ..Default::default()
        })
        .test_id(&format!("{ID}-range-{}", range.id()))
        .hover(|s| s.bg(theme::SURFACE))
        .active(|s| s.bg(theme::BORDER))
        .style_when(plotted, |s| s.text_color(theme::ACCENT))
        .on_press(move || {
            open.set(false);
            let settings = saved.get_untracked();
            if settings.range() != range {
                tab.save(&settings.with_range(range));
            }
        })
        .child(text(range.label()).number_of_lines(1).tabular_nums())
}

/// The strip under the plot: the chains the value is on, which take what
/// room there is, then DefiLlama's credit.
fn strip(session: &Rc<ProtocolTvlSession>) -> View {
    let tvl = session.tvl;
    let chains = move || {
        tvl.with(|tvl| match tvl {
            Some(tvl) => match session::chains_line(&tvl.0) {
                line if line.is_empty() => "No chain breakdown".to_owned(),
                line => line,
            },
            None => String::new(),
        })
    };
    let detail = move || {
        tvl.with(|tvl| {
            tvl.as_ref()
                .map(|tvl| session::chains_detail(&tvl.0))
                .filter(|detail| !detail.is_empty())
                .unwrap_or_else(|| "DefiLlama gives no chain breakdown.".to_owned())
        })
    };
    let page = move || {
        tvl.with_untracked(|tvl| {
            tvl.as_ref()
                .map_or_else(|| SOURCE_URL.to_owned(), |tvl| tvl.0.source_url.clone())
        })
    };
    view()
        .h(px(STRIP_HEIGHT))
        .w_full()
        .flex_none()
        .flex_row()
        .border_t_1()
        .border_color(theme::BORDER)
        .bg(theme::SURFACE)
        .text_size(px(11.0))
        .text_color(theme::TEXT_MUTED)
        .child(
            view()
                .h_full()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .items_center()
                .px(px(10.0))
                .tooltip(ui::hint(detail))
                .test_id(&format!("{ID}-chains"))
                .child(text(chains).min_w(px(0.0)).truncate()),
        )
        .child(source_credit("DefiLlama", page).test_id(&format!("{ID}-credit")))
}
