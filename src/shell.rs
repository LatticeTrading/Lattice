//! The shared Lattice shell. NSH mounts this same Rust tree on desktop and web.

pub(crate) mod everything_bar;

use nsh_core::prelude::*;

use crate::{layout, storage, theme};

/// The root shell: Everything Bar above the persistent workspace grid.
pub fn shell() -> impl IntoElement {
    let mut saved = storage::load();
    layout::normalize(&mut saved);
    if let Err(error) = storage::save(&saved) {
        log::warn!("Lattice: unable to save workspace: {error}");
    }
    let workspace = signal(saved);

    view()
        .flex_1()
        .flex_col()
        .p(px(8.0))
        .bg(theme::background())
        .text_color(theme::foreground())
        .font_family("IBM Plex Mono")
        .child(everything_bar::bar(workspace))
        .child(layout::workspace(workspace))
}
