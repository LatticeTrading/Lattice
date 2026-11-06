//! Application lifecycle and native window configuration, not product state.

use std::{cell::RefCell, rc::Rc};

use anyhow::Context as _;
use gpui_kit::{App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};

use crate::{assets::AppAssets, shell::Shell, theme};

pub fn window_options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1280.), px(800.)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some("Lattice".into()),
            appears_transparent: true,
            ..Default::default()
        }),
        window_min_size: Some(size(px(640.), px(360.))),
        app_id: Some("Lattice".into()),
        ..Default::default()
    }
}

fn start(cx: &mut App) -> anyhow::Result<()> {
    gpui_kit::init(cx);
    theme::init(cx)?;
    // A single-window app must also quit when the OS closes its last window.
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
    gpui_kit::open_window(window_options(cx), cx, |window, cx| {
        cx.new(|cx| Shell::new(window, cx))
    })
    .context("opening the Lattice window")?;
    cx.activate(true);
    Ok(())
}

pub fn run() -> anyhow::Result<()> {
    // Application::run requires a 'static callback. Preserve startup errors so
    // cargo run / a launcher gets a nonzero exit rather than a silent blank app.
    let startup_error = Rc::new(RefCell::new(None));
    let error_slot = startup_error.clone();
    gpui_kit::application()
        .with_assets(AppAssets)
        .run(move |cx| {
            if let Err(error) = start(cx) {
                *error_slot.borrow_mut() = Some(error);
                cx.quit();
            }
        });
    match startup_error.borrow_mut().take() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
