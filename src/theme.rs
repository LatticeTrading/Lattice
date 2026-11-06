//! The initial dark palette, translated from lattice-pro's canonical theme.css.
//! Keep shell colors here; do not introduce a second component/theme system.

use std::borrow::Cow;

use anyhow::Context as _;
use gpui_kit::{
    App, Hsla,
    component::{Theme, ThemeMode},
    px, rgb,
};

pub const FONT_FAMILY: &str = "IBM Plex Mono";
pub const WINDOW_BACKGROUND: u32 = 0x0a0a0a;
pub const BAR_BACKGROUND: u32 = 0x1a1a1a;
pub const BORDER: u32 = 0x2a2a2a;
pub const TEXT_PRIMARY: u32 = 0xffffff;
pub const TEXT_SECONDARY: u32 = 0xa0a0a0;
pub const TEXT_TERTIARY: u32 = 0x707070;
pub const ACCENT: u32 = 0x3b82f6;

pub fn color(value: u32) -> Hsla {
    rgb(value).into()
}

pub fn init(cx: &mut App) -> anyhow::Result<()> {
    // Embedded fonts avoid machine-dependent fallbacks and runtime downloads.
    cx.text_system()
        .add_fonts(vec![
            Cow::Borrowed(include_bytes!(
                "../assets/fonts/ibm-plex-mono/IBMPlexMono-Regular.ttf"
            )),
            Cow::Borrowed(include_bytes!(
                "../assets/fonts/ibm-plex-mono/IBMPlexMono-Medium.ttf"
            )),
            Cow::Borrowed(include_bytes!(
                "../assets/fonts/ibm-plex-mono/IBMPlexMono-SemiBold.ttf"
            )),
            Cow::Borrowed(include_bytes!(
                "../assets/fonts/ibm-plex-mono/IBMPlexMono-Bold.ttf"
            )),
        ])
        .context("registering the bundled IBM Plex Mono fonts")?;

    Theme::change(ThemeMode::Dark, None, cx);
    let theme = Theme::global_mut(cx);
    theme.font_family = FONT_FAMILY.into();
    theme.mono_font_family = FONT_FAMILY.into();
    theme.font_size = px(12.);
    theme.mono_font_size = px(12.);
    theme.radius = px(0.);
    theme.radius_lg = px(0.);
    theme.focus_ring = false; // Kit draws an inset focus line, not a clipped outer ring.
    theme.background = color(WINDOW_BACKGROUND);
    theme.foreground = color(TEXT_PRIMARY);
    theme.border = color(BORDER);
    theme.muted_foreground = color(TEXT_TERTIARY);
    theme.primary = color(ACCENT);
    theme.ring = color(ACCENT);
    theme.title_bar = color(BAR_BACKGROUND);
    theme.title_bar_border = color(BORDER);
    Theme::sync_base(cx);
    Ok(())
}
