//! A market's type and detail tags, as the market picker's results show
//! them.

use nsh_core::prelude::*;

use crate::theme;

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
