//! The credit a widget gives the outside source of its data, where that
//! source asks to be named (DefiLlama): a cell at the far end of the
//! widget's strip that says the source and opens its page in the browser.

use nsh_core::prelude::*;

use crate::{theme, ui};

/// The credit cell of a strip: `name`, as high as the strip, with the
/// strip's line at its left. A press opens `page` (as it is when pressed)
/// outside the app; the page is the caller's to vouch for.
pub(crate) fn source_credit(name: &'static str, page: impl Fn() -> String + 'static) -> View {
    view()
        .h_full()
        .flex_none()
        .flex()
        .items_center()
        .px(px(10.0))
        .border_l_1()
        .border_color(theme::BORDER)
        .text_color(theme::TEXT_MUTED)
        .role(AccessibilityRole::Link)
        .label(format!("Open {name}"))
        .tooltip(ui::hint(move || {
            format!("Data from {name}. Opens its page in the browser.")
        }))
        .hover(|s| s.bg(theme::PANEL).text_color(theme::TEXT))
        .active(|s| s.bg(theme::BORDER))
        .on_press(move || {
            // Whether it opened changes nothing here.
            drop(open_url(page()));
        })
        .child(text(name).number_of_lines(1))
}
