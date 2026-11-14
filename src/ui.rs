//! Shared Lattice controls: text fields, flat buttons, tooltips, settings
//! rows ([`form`]), movable floating panels ([`panel`]) and engine-driven
//! list scrolling ([`RowScroll`]).
//!
//! NSH's `input` is a complete text field on every host (on Windows the
//! system's RichEdit text box, on the web an `<input>`), so typing, caret
//! movement, selection, undo, clipboard shortcuts, and IME composition need no
//! workarounds. A controlled `value` is safe while typing fast: the host drops
//! values based on an older edit. What these helpers add is Lattice's look and
//! one consistent way to bind a field to a signal, so the Layout Manager, the
//! symbol search, and future widgets all behave the same way.

mod form;
mod panel;
mod row_scroll;

pub use form::{check_row, dropdown, label_column_row, label_edge_list_row, label_edge_row};
pub use panel::{Side, centered_panel, floating_panel, modal_panel, settings_panel};
pub use row_scroll::RowScroll;

use nsh_core::{
    element::{Input, input},
    prelude::*,
};

use crate::theme;

/// The default height of a single-line field or a panel row.
pub const ROW_HEIGHT: Points = px(36.0);

/// A single-line text field bound to `value`.
///
/// The field is controlled: every edit writes `value`, and writing `value`
/// from code (clearing it, filling a generated key) updates the field.
/// Chain `placeholder`, `label`, `on_submit`, or more `on_change` listeners
/// onto the returned `Input` as needed.
pub fn text_field(value: Signal<String>) -> Input {
    input()
        .value(move || value.get())
        .on_change(move |text: &str| value.set(text.to_owned()))
        .h(ROW_HEIGHT)
        .min_w(px(0.0))
        .px(px(10.0))
        .text_size(px(11.0))
        .text_color(theme::TEXT)
        .bg(theme::BACKGROUND)
        .placeholder_color(theme::TEXT_FAINT)
        .caret_color(theme::ACCENT)
        // Windows' text box takes the selection as plain RGB, dropping
        // alpha, so the tint is laid over the field's background up front.
        .selection_color(|| {
            nsh_core::element::color(theme::BACKGROUND.get())
                .blend(nsh_core::element::color(theme::ACCENT.opacity(0.35).get()))
        })
        .auto_correct(false)
        .auto_capitalize(AutoCapitalize::None)
}

/// A tooltip's box: `content` in small text on a panel (an element's
/// `.tooltip(..)`).
pub fn hint(content: impl Fn() -> String + 'static) -> View {
    view()
        .max_w(px(420.0))
        .px(px(6.0))
        .py(px(4.0))
        .bg(theme::PANEL)
        .border_1()
        .border_color(theme::BORDER)
        .text_size(px(10.0))
        .text_color(theme::TEXT)
        .child(text(content))
}

/// How a [`button`] reads.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Primary text: ordinary actions.
    Normal,
    /// Muted text that turns red on hover: a destructive row action.
    Quiet,
    /// Red text: confirming something destructive.
    Danger,
}

/// A flat, borderless text button that fills its row's height, with hover,
/// press, focus, and disabled states.
pub fn button(label: impl Into<String>, tone: Tone) -> View {
    let color = match tone {
        Tone::Normal => theme::TEXT,
        Tone::Quiet => theme::TEXT_MUTED,
        Tone::Danger => theme::DANGER,
    };
    view()
        .h_full()
        .flex()
        .items_center()
        .px(px(10.0))
        .text_size(px(11.0))
        .text_color(color)
        .role(AccessibilityRole::Button)
        .hover(move |style| {
            let style = style.bg(theme::SURFACE);
            if tone == Tone::Quiet {
                style.text_color(theme::DANGER)
            } else {
                style
            }
        })
        .active(|style| style.bg(theme::BORDER))
        .disabled_style(|style| style.opacity(0.5))
        .child(text(label.into()))
}
