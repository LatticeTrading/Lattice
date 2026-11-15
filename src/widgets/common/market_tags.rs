//! A market's type and detail tags: in boxes, as the market picker's
//! results show them ([`market_tags`]), or as the segments of a row or a
//! box ([`tag_segment`]).

use nsh_core::prelude::*;

use crate::theme::{self, Role};

/// A market's tags as the old `MarketTags` drew them: its type (`PERP`),
/// then its detail (`USDT-M`, an expiry) in the warning color, each in a
/// small bordered box; the detail's box is left out while it has none.
pub(crate) fn market_tags(
    kind: impl Fn() -> String + 'static,
    detail: impl Fn() -> Option<String> + Clone + 'static,
) -> View {
    let tag = |content: Text| {
        tag_text(content)
            .flex_none()
            .min_h(px(16.0))
            .px(px(5.0))
            .py(px(1.0))
            .border_1()
            .border_color(theme::BORDER)
            .bg(theme::BACKGROUND)
    };
    let shown = detail.clone();
    view()
        .flex_none()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .child(tag(text(kind)).text_color(theme::TEXT_FAINT))
        .child(
            tag(text(move || detail().unwrap_or_default()))
                .text_color(theme::WARNING)
                .style_when(move || shown().is_none(), |s| s.hidden()),
        )
}

/// A market tag as a segment of what it is in (a settings panel's market
/// box, the Price Delta Monitor's rows; the Everything Bar's results draw
/// theirs the same way): no box of its own, a divider before it, and as
/// high as the row it is added to, whose children stretch.
pub(crate) fn tag_segment(label: String, color: Role) -> View {
    view()
        .flex_none()
        .flex()
        .items_center()
        .px(px(8.0))
        .border_l_1()
        .border_color(theme::BORDER)
        .child(tag_text(text(label)).text_color(color))
}

/// A market tag's text as the old `.market-tag` set it: small, semibold,
/// spaced uppercase on one line. The caller picks its color (and its box,
/// if any).
pub(crate) fn tag_text(content: Text) -> Text {
    content
        .text_size(px(9.0))
        .font_weight(FontWeight::SEMIBOLD)
        .letter_spacing(px(0.4))
        .uppercase()
        .number_of_lines(1)
}
