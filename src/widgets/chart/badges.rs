//! The badges that float on the chart's plot beside its header, right of
//! the exchange it ends with: the timeframe's ([`super::timeframe`]),
//! right of it the indicators' ([`super::indicators`]), and right of that
//! fast mode's ([`super::fast`]). The first two say what they are for and
//! open a menu below them ([`floating`]); the third turns its mode on and
//! off ([`switch`]). Each is small, bordered and see-through until the
//! pointer is over it, as the badge that opens the drawing toolbar is.
//! They are laid over the plot's view, after it, so a press on one is its
//! own. In a pane too narrow for the header they stay inside the plot's
//! area, over the header's end.

use nsh_core::{Checked, prelude::*};

use crate::{
    theme,
    widgets::common::plot::{self, Plot},
};

/// A badge's height, how far right of the header's end the first one is,
/// and how far apart they are.
const HEIGHT: f32 = 16.0;
const GAP: f32 = 8.0;
const SPACE: f32 = 4.0;
/// A badge's text size, a character's width at it (the app's font is 0.6
/// of its size wide everywhere) and the room either side of what it says.
const TEXT_SIZE: f32 = 10.0;
const CHARACTER: f32 = 6.0;
const PADDING: f32 = 6.0;
/// How opaque a badge is until the pointer is over it.
const OPACITY: f32 = 0.6;

/// How wide a badge saying `characters` characters is, with `beside`
/// points of something else beside them (an icon and its gap).
pub(super) const fn width(characters: usize, beside: f32) -> f32 {
    2.0 * PADDING + beside + characters as f32 * CHARACTER
}

/// Where the badges start on `plot` (the chart's), in points from its
/// view's left edge: the first beside the header saying `title` and on a
/// whole point, each of the others right of the one before it, as wide as
/// `widths` says each is, and none past the plot's area.
pub(super) fn lefts<const N: usize>(
    plot: &Plot,
    title: Memo<String>,
    widths: impl Fn() -> [f32; N] + 'static,
) -> Memo<[f32; N]> {
    let size = plot.size;
    let (area_left, _) = plot::AREA_ORIGIN;
    computed(move || {
        let beside = (title.with(|title| plot::header_end(title)) + GAP).ceil();
        let (width, _) = size.get();
        let widths = widths();
        let all = widths.iter().sum::<f32>() + SPACE * N.saturating_sub(1) as f32;
        let mut left = beside.min(area_left + width - all).max(area_left);
        widths.map(|badge| {
            let at = left;
            left += badge + SPACE;
            at
        })
    })
}

/// A badge `width` points wide, `left` points from the plot's view's left
/// edge and level with its header: a button, in the accent while `lit`.
/// Its caller names it, gives it what it says and what a press does.
fn badge(
    left: impl Fn() -> f32 + 'static,
    width: impl Fn() -> f32 + 'static,
    lit: impl Fn() -> bool + 'static,
) -> View {
    let (_, top) = plot::HEADER_ORIGIN;
    view()
        .absolute()
        .left(move || px(left()))
        .top(px(top))
        .w(move || px(width()))
        .h(px(HEIGHT))
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::SURFACE.opacity(OPACITY))
        .border_1()
        .border_color(theme::BORDER)
        .text_size(px(TEXT_SIZE))
        .text_color(theme::TEXT_MUTED)
        .role(AccessibilityRole::Button)
        .hover(|s| {
            s.bg(theme::SURFACE)
                .border_color(theme::TEXT_FAINT)
                .text_color(theme::TEXT)
        })
        .active(|s| s.bg(theme::BORDER))
        .style_when(lit, |s| {
            s.bg(theme::SURFACE)
                .border_color(theme::ACCENT)
                .text_color(theme::ACCENT)
        })
}

/// A badge (`width` points wide, `left` points from the plot's view's left
/// edge) that opens and closes what `open` says is open (its menu), lit
/// while it is. Its caller names it and gives it what it says.
pub(super) fn floating(
    left: impl Fn() -> f32 + 'static,
    width: impl Fn() -> f32 + 'static,
    open: Signal<bool>,
) -> View {
    badge(left, width, move || open.get())
        .accessibility_state(move || AccessibilityState {
            expanded: Some(open.get()),
            has_popup: true,
            ..Default::default()
        })
        .on_press(move || open.update(|open| *open = !*open))
}

/// A badge (`width` points wide, `left` points from the plot's view's left
/// edge) that is `on` or off, lit while it is on; `press` hears a press on
/// it. Its caller names it and gives it what it says.
pub(super) fn switch(
    left: impl Fn() -> f32 + 'static,
    width: impl Fn() -> f32 + 'static,
    on: impl Fn() -> bool + Copy + 'static,
    press: impl Fn() + 'static,
) -> View {
    badge(left, width, on)
        .accessibility_state(move || AccessibilityState {
            checked: Some(if on() { Checked::True } else { Checked::False }),
            ..Default::default()
        })
        .on_press(press)
}
