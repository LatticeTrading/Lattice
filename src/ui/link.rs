//! The link button: a dot in a link color ([`crate::link`]) that opens a
//! dropdown of the colors to choose from. The Everything Bar has one (where
//! its search sends a pick) and so has the pane header of a widget that can
//! be linked (the old app's `ColorLinkButton`, with the look of Lattice's
//! own dropdowns: a square bordered panel of 30pt rows, the chosen one named
//! in the accent color).

use nsh_core::{
    overlay::{Align, Placement},
    prelude::*,
};

use super::Side;
use crate::{link::LinkColor, theme};

/// The dropdown's narrowest.
const MENU_WIDTH: f32 = 150.0;
/// A dropdown row's height, as a select's.
const ROW_HEIGHT: f32 = 30.0;
/// The dot beside a dropdown row's name.
const ROW_DOT: f32 = 8.0;

/// A dot in `color`, `size` across: filled with the color, or a hollow
/// ring while it is none.
fn link_dot(color: impl Fn() -> Option<LinkColor> + Clone + 'static, size: f32) -> View {
    let fill = color.clone();
    view()
        .size(px(size))
        .flex_none()
        .rounded_full()
        .border_1()
        .bg(move || match fill() {
            Some(color) => color.role().get(),
            None => theme::TEXT_FAINT.opacity(0.0).get(),
        })
        .border_color(move || match color() {
            Some(color) => color.role().get(),
            None => theme::TEXT_FAINT.get(),
        })
}

/// A link button: a cell showing `current`'s dot (`dot` across), which
/// opens a dropdown of the link colors below it, lined up with its `side`
/// edge. The caller sizes the cell and gives it its borders.
///
/// - `name` names it for assistive tech (`Widget link color: Blue`).
/// - `none` puts "None" (linked to no color) first in the dropdown.
/// - `note(choice)` is small text at a row's far edge (how many widgets a
///   color has); empty for nothing.
/// - `on_pick(choice)` hears the row pressed; the dropdown closes itself,
///   as it does on Escape and on a press outside it.
pub fn link_button(
    name: &'static str,
    current: impl Fn() -> Option<LinkColor> + Copy + 'static,
    none: bool,
    note: impl Fn(Option<LinkColor>) -> String + Copy + 'static,
    on_pick: impl Fn(Option<LinkColor>) + Clone + 'static,
    side: Side,
    dot: f32,
) -> View {
    let open = signal(false);
    let anchor = element_ref();
    let menu = dynamic(move || {
        open.get().then(|| {
            let choices = none
                .then_some(None)
                .into_iter()
                .chain(LinkColor::ALL.map(Some));
            let rows: Vec<View> = choices
                .map(|choice| {
                    let on_pick = on_pick.clone();
                    row(choice, current, note, move || {
                        open.set(false);
                        on_pick(choice);
                    })
                })
                .collect();
            // In an overlay, so it takes its presses wherever it hangs.
            overlay()
                .anchor(&anchor)
                .placement(Placement::Bottom)
                .align(match side {
                    Side::Left => Align::Start,
                    Side::Right => Align::End,
                })
                .offset(px(1.0))
                .on_press_outside(move |_: &Point<Points>| open.set(false))
                .on_cancel(move || open.set(false))
                .child(
                    view()
                        .test_id("link-menu")
                        .min_w(px(MENU_WIDTH))
                        .flex_col()
                        .py(px(4.0))
                        .bg(theme::PANEL)
                        .border_1()
                        .border_color(theme::BORDER)
                        .text_size(px(11.0))
                        .text_color(theme::TEXT)
                        .role(AccessibilityRole::Listbox)
                        .label(name)
                        .children(rows),
                )
        })
    });
    view()
        .element_ref(&anchor)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .role(AccessibilityRole::Button)
        .label(move || format!("{name}: {}", current().map_or("None", LinkColor::label)))
        .accessibility_state(move || AccessibilityState {
            expanded: Some(open.get()),
            has_popup: true,
            ..Default::default()
        })
        .hover(|s| s.bg(theme::BACKGROUND))
        .active(|s| s.bg(theme::BORDER))
        .on_press(move || open.update(|open| *open = !*open))
        .child(link_dot(current, dot))
        .child(menu)
}

/// One choice of the dropdown: its dot, its name (in the accent color while
/// it is the current one) and its note.
fn row(
    choice: Option<LinkColor>,
    current: impl Fn() -> Option<LinkColor> + Copy + 'static,
    note: impl Fn(Option<LinkColor>) -> String + Copy + 'static,
    on_press: impl Fn() + 'static,
) -> View {
    let selected = move || current() == choice;
    let label = choice.map_or("None", LinkColor::label);
    view()
        .h(px(ROW_HEIGHT))
        .w_full()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(10.0))
        .role(AccessibilityRole::Option)
        .label(label)
        .accessibility_state(move || AccessibilityState {
            selected: selected(),
            ..Default::default()
        })
        .hover(|s| s.bg(theme::SURFACE))
        .active(|s| s.bg(theme::BORDER))
        .on_press(on_press)
        .child(link_dot(move || choice, ROW_DOT))
        .child(
            text(label)
                .flex_1()
                .min_w(px(0.0))
                .number_of_lines(1)
                .text_color(move || {
                    if selected() {
                        theme::ACCENT.get()
                    } else {
                        theme::TEXT.get()
                    }
                }),
        )
        .child(
            text(move || note(choice))
                .flex_none()
                .text_size(px(10.0))
                .text_color(theme::TEXT_FAINT),
        )
}
