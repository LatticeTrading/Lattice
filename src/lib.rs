//! One Rust UI, mounted by NSH's native runners and its WebAssembly runtime.

pub mod app;
mod assets;
pub mod data;
mod exchanges;
mod format;
mod layout;
mod link;
mod notify;
mod shell;
mod storage;
mod theme;
mod ui;
mod widgets;

use nsh_core::prelude::*;

#[native_module("lattice/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>().assets(assets!());
}

/// Since NSH 0.7 the app host never shows on the desktops: the app declares
/// the window its UI is in. Off the desktops (the web) the window is the
/// page's one surface.
#[component]
pub fn App() -> impl IntoElement {
    nsh_core::element::desktop::window()
        .id("main")
        .title("Lattice")
        .default_size(size(px(1280.0), px(800.0)))
        .restore_bounds(true)
        .content(shell::shell)
}
