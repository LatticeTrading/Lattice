//! Lattice's shared NSH design tokens (also loaded by the CLI from theme.json).
use nsh_base::{Theme, theme};
use nsh_core::prelude::*;

pub fn tokens() -> Tokens {
    Theme::from_json(include_str!("../theme.json"))
        .expect("valid bundled Lattice theme")
        .into_tokens()
}

pub fn background() -> AdaptiveColor {
    theme().background
}
pub fn surface() -> AdaptiveColor {
    theme().card
}
pub fn border() -> AdaptiveColor {
    theme().border
}
pub fn foreground() -> AdaptiveColor {
    theme().foreground
}
pub fn secondary() -> AdaptiveColor {
    theme().muted_foreground
}
pub fn tertiary() -> AdaptiveColor {
    theme()
        .color("tertiary")
        .unwrap_or(AdaptiveColor::TRANSPARENT)
}
pub fn accent() -> AdaptiveColor {
    theme().primary
}
