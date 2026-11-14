//! A market as a settings panel shows it: the old app's symbol chooser box.

use nsh_core::prelude::*;

use super::{LogoStyle, exchange_logo, tag_text};
use crate::{data::ferris::MarketSelection, exchanges, theme};

/// The box's height, and its logo segment's width.
const HEIGHT: f32 = 28.0;
const LOGO_SEGMENT: f32 = 24.0;
/// How the box draws its exchange's logo: initials without a box, the
/// segment is one.
const LOGO: LogoStyle = LogoStyle {
    size: 14.0,
    color: theme::TEXT_MUTED,
    initials_size: 7.0,
    boxed: false,
};

/// `market` in the old settings panel's chooser box
/// (`.settings-symbol-search-btn`): bordered, in segments (its exchange's
/// logo, its pair, its type, and its detail when it has one), lit up while
/// hovered. The caller makes it a button (its role, label and press) and
/// sizes it; left alone it is as wide as its segments.
pub(crate) fn market_chooser(market: &MarketSelection) -> View {
    // The old `.market-tag` segments: no box of their own, a divider
    // before each.
    let segment = |label: String, color: theme::Role| {
        view()
            .flex_none()
            .flex()
            .items_center()
            .px(px(8.0))
            .border_l_1()
            .border_color(theme::BORDER)
            .child(tag_text(text(label)).text_color(color))
    };
    let chooser = view()
        .min_h(px(HEIGHT))
        .flex_row()
        .bg(theme::SURFACE)
        .border_1()
        .border_color(theme::BORDER)
        // Raised while hovered, so its accent edge draws over a box laid
        // against it (the Trade Stream's ×, which shares its edge).
        .relative()
        .hover(|s| s.bg(theme::PANEL).border_color(theme::ACCENT).z_index(1))
        .child(
            view()
                .w(px(LOGO_SEGMENT))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .border_r_1()
                .border_color(theme::BORDER)
                .overflow_hidden()
                .child(exchange_logo(&market.exchange, LOGO)),
        )
        .child(
            view()
                .flex_shrink(1.0)
                .min_w(px(0.0))
                .max_w(px(92.0))
                .flex()
                .items_center()
                .px(px(8.0))
                .child(
                    text(market.symbol.clone())
                        .min_w(px(0.0))
                        .truncate()
                        .text_size(px(11.0))
                        .text_color(theme::TEXT),
                ),
        )
        .child(segment(market.kind.label().to_owned(), theme::TEXT_FAINT));
    match market.detail() {
        Some(detail) => chooser.child(segment(detail, theme::WARNING)),
        None => chooser,
    }
}

/// What names `market` for assistive tech: `Bybit BTC/USDT PERP USDT-M`.
pub(crate) fn market_name(market: &MarketSelection) -> String {
    format!(
        "{} {} {}",
        exchanges::display_name(&market.exchange),
        market.symbol,
        market.tags()
    )
}
