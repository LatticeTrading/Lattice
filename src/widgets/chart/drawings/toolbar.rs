//! The drawing toolbar: a narrow strip down the plot's left side, and the
//! badge that brings it out.
//!
//! Open, it is a column of buttons, an icon each: the tools
//! ([`Tool::ALL`]; the one in hand is lit, and pressing it again goes back
//! to the cursor), under a rule removing the market's drawings, which asks
//! for a second press, and at its bottom the button that closes it.
//!
//! Closed, it takes no room, and nothing beside the plot is its own: the
//! plot has the chart's whole width, and a thin see-through badge floats
//! on it ([`toolbar_badge`]), just inside its left edge and halfway down
//! it, over the bars there. A press on the badge opens the toolbar again.
//! Whether the toolbar is open is saved in the chart's settings.

use std::rc::Rc;

use nsh_core::{element::ImageSrc, prelude::*};

use super::{Editor, Tool};
use crate::{
    theme, ui,
    widgets::{
        TabContext,
        chart::ChartSettings,
        common::plot::{self, Plot},
    },
};

/// The toolbar's width.
const WIDTH: f32 = 28.0;
/// A button's height, and the height of the one at the toolbar's bottom
/// that closes it.
const BUTTON: f32 = 26.0;
const CLOSE: f32 = 24.0;
/// An icon's side, and a chevron's.
const ICON: f32 = 14.0;
const CHEVRON: f32 = 10.0;
/// The badge: how wide and tall it is, how far inside the plot's left
/// edge, and its chevron's side.
const BADGE_WIDTH: f32 = 10.0;
const BADGE_HEIGHT: f32 = 20.0;
const BADGE_INSET: f32 = 4.0;
const BADGE_CHEVRON: f32 = 8.0;
/// How opaque the badge is until the pointer is over it.
const BADGE_OPACITY: f32 = 0.6;

/// What the badge and the button that closes the toolbar are named.
const SHOW: &str = "Show drawing tools";
const HIDE: &str = "Hide drawing tools";

/// The toolbar of `editor`'s tools, for the chart in `tab`: beside the
/// plot while it is open, nothing while it is closed.
pub(crate) fn toolbar(editor: &Rc<Editor>, tab: &TabContext<ChartSettings>) -> View {
    let saved = tab.settings;
    let open = computed(move || saved.with(ChartSettings::tools));
    let tools = Tool::ALL.map(|tool| tool_button(editor.clone(), tool));
    // Cut short in a pane too low for them all, so the button under them
    // that closes the toolbar always shows.
    let column = view()
        .w_full()
        .flex_1()
        .min_h(px(0.0))
        .flex_col()
        .overflow_hidden()
        .children(tools)
        .child(
            view()
                .h(px(1.0))
                .mx(px(5.0))
                .my(px(3.0))
                .flex_none()
                .bg(theme::BORDER),
        )
        .child(remove_button(editor.clone()));
    view()
        .w(px(WIDTH))
        .h_full()
        .flex_none()
        .flex_col()
        .overflow_hidden()
        .bg(theme::BACKGROUND)
        .border_r_1()
        .border_color(theme::BORDER)
        .test_id("chart-tools")
        .style_when(move || !open.get(), |s| s.hidden())
        .child(column)
        .child(close_button(editor.clone(), tab.clone()))
}

/// The badge that opens the toolbar, floating on `plot` (the chart's)
/// while the toolbar is closed: just inside the plot's left edge and
/// halfway down it, thin, bordered and see-through until the pointer is
/// over it. It is laid over the plot's view, after it, so it is over the
/// plot's surfaces and a press on it is its own.
pub(crate) fn toolbar_badge(tab: &TabContext<ChartSettings>, plot: &Plot) -> View {
    let (saved, tab) = (tab.settings, tab.clone());
    let open = computed(move || saved.with(ChartSettings::tools));
    let size = plot.size;
    let (left, top) = plot::AREA_ORIGIN;
    view()
        .absolute()
        .left(px(left + BADGE_INSET))
        .top(move || {
            // Halfway down the plot, on a whole point, and never above it.
            let (_, height) = size.get();
            px(top + ((height - BADGE_HEIGHT) / 2.0).floor().max(0.0))
        })
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
        .label(SHOW)
        .accessibility_state(|| AccessibilityState {
            expanded: Some(false),
            ..Default::default()
        })
        .test_id("chart-tools-show")
        .tooltip(ui::hint(|| SHOW.to_owned()))
        .hover(|s| {
            s.bg(theme::SURFACE)
                .border_color(theme::TEXT_FAINT)
                .text_color(theme::TEXT)
        })
        .active(|s| s.bg(theme::BORDER))
        .style_when(move || open.get(), |s| s.hidden())
        .on_press(move || tab.save(&saved.get_untracked().with_tools(true)))
        .child(icon(
            include_str!("../../../../assets/icons/chevron-right.svg"),
            BADGE_CHEVRON,
        ))
}

/// A button of the toolbar: `icon` in its middle, lit by hover and press.
fn button(icon: View) -> View {
    view()
        .w_full()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .text_color(theme::TEXT_MUTED)
        .role(AccessibilityRole::Button)
        .hover(|s| s.bg(theme::SURFACE).text_color(theme::TEXT))
        .active(|s| s.bg(theme::BORDER))
        .child(icon)
}

/// `markup` (an SVG's) as an icon.
fn icon(markup: &'static str, side: f32) -> View {
    view()
        .size(px(side))
        .flex_none()
        .child(svg(ImageSrc::svg_markup(markup)).size_full())
}

/// The toolbar's bottom button, which closes it. That puts the tool in
/// hand away: nothing would show which one it is.
fn close_button(editor: Rc<Editor>, tab: TabContext<ChartSettings>) -> View {
    let saved = tab.settings;
    button(icon(
        include_str!("../../../../assets/icons/chevron-left.svg"),
        CHEVRON,
    ))
    .h(px(CLOSE))
    .border_t_1()
    .border_color(theme::BORDER)
    .text_color(theme::TEXT_FAINT)
    .label(HIDE)
    .accessibility_state(|| AccessibilityState {
        expanded: Some(true),
        ..Default::default()
    })
    .test_id("chart-tools-hide")
    .tooltip(ui::hint(|| HIDE.to_owned()))
    .on_press(move || {
        editor.take(Tool::Cursor);
        tab.save(&saved.get_untracked().with_tools(false));
    })
}

/// A tool's button, lit while the tool is in hand.
fn tool_button(editor: Rc<Editor>, tool: Tool) -> View {
    let (state, lit) = (editor.clone(), editor.clone());
    button(icon(tool.icon(), ICON))
        .h(px(BUTTON))
        .style_when(
            move || lit.tool() == tool,
            |s| s.text_color(theme::ACCENT).bg(theme::ACCENT.opacity(0.12)),
        )
        .label(tool.label())
        .accessibility_state(move || AccessibilityState {
            selected: state.tool() == tool,
            ..Default::default()
        })
        .test_id(&format!("chart-tool-{}", tool.id()))
        .tooltip(ui::hint(move || tool.label().to_owned()))
        .on_press(move || editor.take(tool))
}

/// "Remove drawings": every drawing of the market charted goes on its
/// second press (the first turns it red and says how many would go);
/// leaving it forgets the first.
fn remove_button(editor: Rc<Editor>) -> View {
    let asked = signal(false);
    let (counted, none, removed) = (editor.clone(), editor.clone(), editor);
    let name = move || match (asked.get(), counted.count()) {
        (true, 1) => "Remove 1 drawing?".to_owned(),
        (true, count) => format!("Remove {count} drawings?"),
        _ => "Remove drawings".to_owned(),
    };
    let hint = name.clone();
    button(icon(
        include_str!("../../../../assets/icons/trash.svg"),
        ICON,
    ))
    .h(px(BUTTON))
    .style_when(move || asked.get(), |s| s.text_color(theme::DANGER))
    .label(name)
    .disabled(move || none.count() == 0)
    .disabled_style(|s| s.opacity(0.4))
    .test_id("chart-tools-remove")
    .tooltip(ui::hint(hint))
    .on_mouse_leave(move |_: &MouseEvent| asked.set(false))
    .on_press(move || {
        if asked.get_untracked() {
            removed.remove_all();
        }
        asked.update(|asked| *asked = !*asked);
    })
}
