//! The shared Lattice shell. NSH mounts this same Rust tree on desktop and web.

pub(crate) mod clock;
pub(crate) mod everything_bar;
mod notifications;
mod settings;

use nsh_core::prelude::*;

use crate::{
    data::DataServices,
    layout,
    notify::Notifications,
    storage, theme,
    widgets::{common::accounts::AddressGroups, picker::Picker},
};

/// The root shell: Everything Bar above the persistent workspace grid, and
/// the notifications that pop up over both.
pub fn shell() -> impl IntoElement {
    // Before anything that renders: every widget below finds the data
    // services in its context. Nothing connects until one asks for data.
    let services = DataServices::provide();
    // Before anything that renders too: whatever has something to tell the
    // user finds the center in its context.
    let center = Notifications::provide();
    // The connected account's trades are told whether a widget shows them
    // or not.
    notifications::fills::announce(services.trading.clone(), center);
    let active_theme = theme::ActiveTheme::restore();
    let clock = clock::Clock::restore();
    // Widgets that show times (the Trade Stream) show them as the bar does.
    provide_context(clock);
    // The address groups the account widgets share, whichever one changes
    // them.
    AddressGroups::provide();
    let mut saved = storage::load();
    layout::normalize(&mut saved);
    if let Err(error) = storage::save(&saved) {
        log::warn!("Lattice: unable to save workspace: {error}");
    }
    let workspace = signal(saved);
    // One widget picker, which panes' `+` and the bar's New Widget open.
    let picker = Picker::new();
    // A widget's session lasts while the widget is anywhere in the
    // workspace: closing its tab, removing its pane, clearing or deleting
    // its page, or importing a layout releases it.
    services
        .sessions
        .follow(move || layout::widget_ids(&workspace.get()));

    view()
        .flex_1()
        .flex_col()
        .p(px(8.0))
        .bg(theme::BACKGROUND)
        .text_color(theme::TEXT)
        .font_family("IBM Plex Mono")
        .child(everything_bar::bar(workspace, active_theme, clock, picker))
        .child(layout::workspace(workspace, picker))
        .child(picker.view(workspace))
        .child(notifications::popups(center, clock))
}
