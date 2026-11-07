//! One Rust UI, mounted by NSH's native runners and its WebAssembly runtime.

pub mod app;
mod assets;
mod layout;
mod shell;
mod storage;
mod theme;

use nsh_core::prelude::*;

#[native_module("lattice/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>().assets(assets!());
}

#[component]
pub fn App() -> impl IntoElement {
    shell::shell()
}
