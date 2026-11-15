//! The badges that float on the chart's plot beside its header, right of
//! the exchange it ends with: the timeframe's ([`super::timeframe`]) and,
//! right of it, the indicators' ([`super::indicators`]). Each says what it
//! is for and opens a menu below it; each is small, bordered and
//! see-through until the pointer is over it, as the badge that opens the
//! drawing toolbar is. They are laid over the plot's view, after it, so a
//! press on one is its own. In a pane too narrow for the header they stay
//! inside the plot's area, over the header's end.

use nsh_core::prelude::*;

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

/// Where the two badges start on `plot` (the chart's), in points from its
/// view's left edge: the first, `first` points wide, beside the header
/// saying `title` and on a whole point, the second, `second` points wide,
/// right of it, and neither past the plot's area.
pub(super) fn lefts(
    plot: &Plot,
    title: Memo<String>,
    first: impl Fn() -> f32 + 'static,
    second: f32,
) -> Memo<[f32; 2]> {
    let size = plot.size;
    let (area_left, _) = plot::AREA_ORIGIN;
    computed(move || {
        let beside = (title.with(|title| plot::header_end(title)) + GAP).ceil();
        let (width, _) = size.get();
        let first = first();
        let left = beside
            .min(area_left + width - first - SPACE - second)
            .max(area_left);
        [left, left + first + SPACE]
    })
}

/// A badge `width` points wide, `left` points from the plot's view's left
/// edge and level with its header: a button that opens and closes what
/// `open` says is open (its menu), lit while it is. Its caller names it
/// and gives it what it says.
pub(super) fn floating(
    left: impl Fn() -> f32 + 'static,
    width: impl Fn() -> f32 + 'static,
    open: Signal<bool>,
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
        .accessibility_state(move || AccessibilityState {
            expanded: Some(open.get()),
            has_popup: true,
            ..Default::default()
        })
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
}
