//! The Protocol TVL's picker: the panel its protocol's badge opens below
//! itself, flush blocks top to bottom:
//!
//! - the search field, which asks DefiLlama's list (through Ferris) once
//!   typing has stopped ([`Search`]);
//! - what is to be said of the search, as a strip: that it is asking, that
//!   it found nothing, or why it failed;
//! - the protocols found, a line each: its name, what it is and its TVL.
//!   A parent protocol leads its products ("All products"); before
//!   anything is typed the lines are the exchanges Lattice shows, each
//!   with its logo. The one charted is in the accent color.
//!
//! A press on a line charts it (saved in the tab record), closes the panel
//! and empties the search.

use std::rc::Rc;

use nsh_core::prelude::*;

use super::{
    session::{Choice, Found, ProtocolTvlSession, Search, venue_choices},
    settings::ProtocolTvlSettings,
};
use crate::{
    format, theme,
    ui::{self, Side},
    widgets::{
        TabContext,
        common::{LogoStyle, exchange_logo},
    },
};

/// The panel's width: the widgets' settings panels'.
const PANEL_WIDTH: Points = px(340.0);
/// A line of the list, with the line under it.
const LINE: f32 = 28.0;
/// The lines that show before the list scrolls.
const LINES_SHOWN: f32 = 10.0;
/// How a line draws an exchange's logo.
const LOGO: LogoStyle = LogoStyle {
    size: 14.0,
    color: theme::TEXT,
    initials_size: 7.0,
    boxed: true,
};

/// The picker of `session`'s widget, below `anchor` (its badge).
pub(super) fn panel(
    session: &Rc<ProtocolTvlSession>,
    tab: &TabContext<ProtocolTvlSettings>,
    anchor: &ElementRef,
) -> Overlay {
    let Search { open, text, found } = session.search;
    let saved = tab.settings;
    let current = computed(move || saved.with(|settings| settings.protocol().to_owned()));
    let pick: Rc<dyn Fn(&str)> = {
        let tab = tab.clone();
        Rc::new(move |slug: &str| {
            open.set(false);
            text.set(String::new());
            tab.save(&saved.get_untracked().with_protocol(slug));
        })
    };
    let field = ui::text_field(text)
        .auto_focus(true)
        .placeholder("Search DefiLlama protocols…")
        .label("Search protocols")
        .flex_none()
        .w_full();
    let said = said(move || match found.get() {
        Found::Searching => Some("Searching…".to_owned()),
        Found::Failed(error) => Some(format!("Could not search: {error}")),
        Found::Listed(choices) if choices.is_empty() => Some("No protocol found.".to_owned()),
        Found::Venues | Found::Listed(_) => None,
    });
    let lines = dynamic(move || {
        let choices = match found.get() {
            Found::Venues => venue_choices(),
            Found::Listed(choices) => choices,
            Found::Searching | Found::Failed(_) => Vec::new(),
        };
        let lines = choices
            .into_iter()
            .map(|choice| line(choice, current, pick.clone()));
        view().w_full().flex_col().children(lines)
    });
    let body = view()
        .test_id("protocol-tvl-picker")
        .flex_col()
        .child(field)
        .child(said)
        .child(
            scroll()
                .w_full()
                .max_h(px(LINE * LINES_SHOWN))
                .border_t_1()
                .border_color(theme::BORDER)
                .child(lines),
        );
    ui::settings_panel(
        anchor,
        Side::Left,
        PANEL_WIDTH,
        "Protocol",
        move || open.set(false),
        body,
    )
}

/// A strip under the field saying `content` in small muted text, with the
/// stack's line over it; hidden while there is nothing to say.
fn said(content: impl Fn() -> Option<String> + 'static) -> View {
    let content = computed(content);
    view()
        .flex_none()
        .w_full()
        .px(px(10.0))
        .py(px(5.0))
        .border_t_1()
        .border_color(theme::BORDER)
        .text_size(px(11.0))
        .text_color(theme::TEXT_MUTED)
        .role(AccessibilityRole::Text)
        .test_id("protocol-tvl-picker-said")
        .style_when(move || content.with(Option::is_none), |s| s.hidden())
        .child(text(move || content.get().unwrap_or_default()))
}

/// One protocol of the list: its exchange's logo when it is one Lattice
/// shows, its name, what it is, and its TVL; in the accent color while it
/// is the one charted. A press charts it.
fn line(choice: Choice, current: Memo<String>, pick: Rc<dyn Fn(&str)>) -> View {
    let Choice {
        slug,
        name,
        note,
        tvl,
        exchange,
    } = choice;
    let charted = {
        let slug = slug.clone();
        move || current.with(|current| *current == slug)
    };
    let selected = charted.clone();
    let row = view()
        .h(px(LINE))
        .w_full()
        .flex_none()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(10.0))
        .border_b_1()
        .border_color(theme::BORDER)
        .text_size(px(11.0))
        .text_color(theme::TEXT)
        .role(AccessibilityRole::Button)
        .label(format!("Chart {name}"))
        .accessibility_state(move || AccessibilityState {
            selected: selected(),
            ..Default::default()
        })
        .test_id(&format!("protocol-tvl-pick-{slug}"))
        .hover(|s| s.bg(theme::SURFACE))
        .active(|s| s.bg(theme::BORDER))
        .style_when(charted, |s| s.text_color(theme::ACCENT))
        .on_press(move || pick(&slug));
    let row = match exchange {
        Some(exchange) => row.child(exchange_logo(exchange, LOGO)),
        None => row,
    };
    let row = row.child(text(name).flex_1().min_w(px(0.0)).truncate());
    let row = if note.is_empty() {
        row
    } else {
        row.child(
            text(note)
                .flex_none()
                .number_of_lines(1)
                .text_size(px(10.0))
                .text_color(theme::TEXT_FAINT),
        )
    };
    match tvl {
        Some(tvl) => row.child(
            text(format::compact_usd(tvl))
                .flex_none()
                .number_of_lines(1)
                .tabular_nums()
                .text_color(theme::TEXT_MUTED),
        ),
        None => row,
    }
}
