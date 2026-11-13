//! The shared Lattice shell. NSH mounts this same Rust tree on desktop and web.

mod clock;
pub(crate) mod everything_bar;
mod settings;

use nsh_core::prelude::*;

use crate::{data::DataServices, layout, storage, theme};

/// The root shell: Everything Bar above the persistent workspace grid.
pub fn shell() -> impl IntoElement {
    // Before anything that renders: every widget below finds the data
    // services in its context. Nothing connects until one asks for data.
    let services = DataServices::provide();
    let active_theme = theme::ActiveTheme::restore();
    let clock = clock::Clock::restore();
    let mut saved = storage::load();
    layout::normalize(&mut saved);
    if let Err(error) = storage::save(&saved) {
        log::warn!("Lattice: unable to save workspace: {error}");
    }
    let workspace = signal(saved);
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
        .child(everything_bar::bar(workspace, active_theme, clock))
        .child(layout::workspace(workspace))
}
