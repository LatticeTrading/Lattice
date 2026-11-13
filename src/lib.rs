//! One Rust UI, mounted by NSH's native runners and its WebAssembly runtime.

pub mod app;
mod assets;
pub mod data;
mod exchanges;
mod format;
mod layout;
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

#[component]
pub fn App() -> impl IntoElement {
    shell::shell()
}
