//! An exchange's logo, or its initials when Lattice has none. Each place
//! that shows one draws it in its own size and colors ([`LogoStyle`]).

use nsh_core::prelude::*;

use crate::{
    exchanges,
    theme::{self, Role},
    ui,
};

/// How one place draws exchange logos.
#[derive(Clone, Copy)]
pub(crate) struct LogoStyle {
    /// The side of the logo's square, and of the initials'.
    pub size: f32,
    /// The logo's color; the initials are always muted.
    pub color: Role,
    pub initials_size: f32,
    /// Whether the initials sit in a bordered box.
    pub boxed: bool,
}

impl LogoStyle {
    fn logo(self, logo: Svg) -> Svg {
        logo.size(px(self.size)).text_color(self.color)
    }

    fn initials(self, initials: Text) -> Text {
        // Centered both ways in their square, as the old app's badge
        // (`ExchangeLogo.tsx`, an SVG text at mid-height) drew them.
        let initials = initials
            .size(px(self.size))
            .text_center()
            .inline_style(ui::middle());
        let initials = if self.boxed {
            initials
                .bg(theme::SURFACE)
                .border_1()
                .border_color(theme::BORDER)
        } else {
            initials
        };
        initials
            .text_size(px(self.initials_size))
            .font_weight(FontWeight::BOLD)
            .text_color(theme::TEXT_MUTED)
    }
}

/// `exchange`'s logo, or its initials when Lattice has none.
pub(crate) fn exchange_logo(exchange: &str, style: LogoStyle) -> View {
    match exchanges::logo(exchange) {
        Some(logo) => view().flex_none().child(style.logo(svg(logo))),
        None => view()
            .flex_none()
            .child(style.initials(text(exchanges::initials(exchange)))),
    }
}

/// The logo of an exchange that changes (a virtual list's row bound to
/// another market): the logo and the initials, for the caller to add side
/// by side, each hidden while the other shows. With no exchange, the
/// initials show, empty.
pub(crate) fn bound_exchange_logo(
    exchange: impl Fn() -> Option<String> + Clone + 'static,
    style: LogoStyle,
) -> (Svg, Text) {
    let logo = {
        let exchange = exchange.clone();
        computed(move || exchange().and_then(|id| exchanges::logo(&id)))
    };
    let initials = move || {
        exchange()
            .map(|id| exchanges::initials(&id))
            .unwrap_or_default()
    };
    (
        style
            .logo(svg(move || {
                logo.get().unwrap_or_else(exchanges::blank_logo)
            }))
            .flex_none()
            .style_when(move || logo.with(Option::is_none), |s| s.hidden()),
        style
            .initials(text(initials))
            .flex_none()
            .style_when(move || logo.with(Option::is_some), |s| s.hidden()),
    )
}
