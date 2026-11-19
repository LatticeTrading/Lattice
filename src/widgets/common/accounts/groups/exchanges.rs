//! The exchanges an address is watched on, as a button ([`exchange_button`]):
//! a logo for each, which opens a checklist of the exchanges that report
//! accounts below it. Checking one watches the address there too;
//! unchecking takes it out, but for the last, which cannot be unchecked. The
//! list stays open while exchanges are checked, until a press outside it or
//! Escape.

use std::rc::Rc;

use nsh_core::{
    overlay::{Align, Placement},
    prelude::*,
};

use crate::{
    exchanges, theme, ui,
    widgets::common::{LogoStyle, exchange_logo},
};

/// The exchanges on offer: each its id and its name, in the portfolio's
/// order.
pub(super) type Venues = Rc<[(&'static str, &'static str)]>;

/// How the button and the list draw an exchange's logo.
const LOGO: LogoStyle = LogoStyle {
    size: 14.0,
    color: theme::TEXT,
    initials_size: 7.0,
    boxed: true,
};
/// How far apart the logos of several exchanges are.
const LOGO_GAP: f32 = 3.0;
/// The list's narrowest.
const MENU_WIDTH: f32 = 168.0;

/// The button of the exchanges `watched` says an address is watched on
/// (their ids, tracked), of those on `offer`. A check in its list calls
/// `on_toggle` with the exchange and whether it is now watched. `label`
/// names the button for assistive tech, and `id` is its test id (its
/// list's is `{id}-menu`, a row's `{id}-{exchange}`).
pub(super) fn exchange_button(
    offer: &Venues,
    watched: impl Fn() -> Vec<String> + 'static,
    on_toggle: impl Fn(&'static str, bool) + 'static,
    label: String,
    id: &str,
) -> View {
    let watched = computed(watched);
    let open = signal(false);
    let anchor = element_ref();
    let menu = {
        let (offer, id) = (offer.clone(), id.to_owned());
        let on_toggle: Rc<dyn Fn(&'static str, bool)> = Rc::new(on_toggle);
        dynamic(move || {
            open.get()
                .then(|| menu(&offer, watched, &on_toggle, &anchor, open, &id))
        })
    };
    let logos = dynamic(move || {
        let logos: Vec<View> = watched
            .get()
            .iter()
            .map(|exchange| exchange_logo(exchange, LOGO))
            .collect();
        Some(
            view()
                .flex_none()
                .flex_row()
                .items_center()
                .gap(px(LOGO_GAP))
                .children(logos),
        )
    });
    let names = {
        let offer = offer.clone();
        move || {
            let names: Vec<String> = watched.with(|watched| {
                watched
                    .iter()
                    .map(|exchange| name_of(&offer, exchange))
                    .collect()
            });
            names.join(", ")
        }
    };
    view()
        .element_ref(&anchor)
        .h_full()
        .flex_none()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .px(px(10.0))
        .text_size(px(11.0))
        .role(AccessibilityRole::Button)
        .label(label)
        .test_id(id)
        .accessibility_state(move || AccessibilityState {
            expanded: Some(open.get()),
            has_popup: true,
            ..Default::default()
        })
        .tooltip(ui::hint(names))
        .hover(|style| style.bg(theme::SURFACE))
        .active(|style| style.bg(theme::BORDER))
        .style_when(move || open.get(), |style| style.bg(theme::SURFACE))
        .on_press(move || open.update(|open| *open = !*open))
        .child(logos)
        .child(text("▾").text_color(theme::TEXT_FAINT))
        .child(menu)
}

/// What the exchange `id` is called: its name on `offer`, else the name
/// Lattice knows it by (one a newer build saved).
pub(super) fn name_of(offer: &Venues, id: &str) -> String {
    offer
        .iter()
        .find(|(venue, _)| *venue == id)
        .map_or_else(|| exchanges::display_name(id), |(_, name)| *name)
        .to_owned()
}

/// The checklist below `anchor` (the button), which `open` says is open: in
/// an overlay, so it takes its presses wherever it hangs.
fn menu(
    offer: &Venues,
    watched: Memo<Vec<String>>,
    on_toggle: &Rc<dyn Fn(&'static str, bool)>,
    anchor: &ElementRef,
    open: Signal<bool>,
    id: &str,
) -> Overlay {
    let rows = offer.iter().map(|&(venue, name)| {
        let checked = move || watched.with(|watched| watched.iter().any(|id| id == venue));
        let last = move || watched.with(|watched| watched.len() == 1 && watched[0] == venue);
        let on_toggle = on_toggle.clone();
        let toggle = move || on_toggle(venue, !checked());
        ui::icon_check_row(exchange_logo(venue, LOGO), name, checked, last, toggle)
            .test_id(&format!("{id}-{venue}"))
    });
    overlay()
        .anchor(anchor)
        .placement(Placement::Bottom)
        .align(Align::End)
        .offset(px(2.0))
        .on_press_outside(move |_: &Point<Points>| open.set(false))
        .on_cancel(move || open.set(false))
        .child(
            view()
                .test_id(&format!("{id}-menu"))
                .min_w(px(MENU_WIDTH))
                .flex_col()
                .py(px(4.0))
                .bg(theme::PANEL)
                .border_1()
                .border_color(theme::BORDER)
                .text_color(theme::TEXT)
                .role(AccessibilityRole::Menu)
                .label("Exchanges")
                .children(rows.collect::<Vec<_>>()),
        )
}
