//! Lattice's theme engine: the bundled themes, the color roles the UI draws
//! with, and switching themes while the app runs.
//!
//! A theme is one JSON file in `themes/` (`name`, `appearance`, and a
//! `colors` entry for every role in `roles.rs`), listed in [`BUNDLED`].
//! UI code never names a color, only a role (`theme::SURFACE`,
//! `theme::ACCENT.opacity(0.1)`), and a role is read again whenever the
//! theme changes. Switching themes repaints the open UI in place: panels,
//! focus, and drags carry on.
//!
//! Each theme becomes NSH design tokens, one per role under the role's own
//! name, so roles are Tailwind colors too (`bg-surface`), and NSH's
//! standard token names are drawn from the roles (`roles::NSH_ALIASES`).
//! A theme gives each color the same value in light and dark appearance;
//! `appearance` only picks how the platform draws its own chrome.

mod roles;
#[cfg(test)]
mod tests;

use std::{collections::BTreeMap, sync::LazyLock};

use nsh_core::{
    element::{Tokens, set_color_scheme, set_tokens},
    platform::Appearance,
    prelude::*,
};
use serde::Deserialize;

pub(crate) use roles::*;

use crate::storage;

/// `(id, file)` of the bundled theme `themes/<id>.json`.
macro_rules! bundled {
    ($id:literal) => {
        ($id, include_str!(concat!("../themes/", $id, ".json")))
    };
}

/// The bundled themes, in the order Settings lists and cycles them. The
/// first is the default, which the others fall back on for any role they
/// get wrong. Adding a theme is adding its file to `themes/` and its id
/// here; `cargo test` then checks it gives every role a color. The id is
/// what Settings saves, so a shipped theme's file keeps its name.
const BUNDLED: &[(&str, &str)] = &[
    bundled!("dark"),
    bundled!("light"),
    // The rest alphabetically, so a long dropdown is easy to scan.
    bundled!("catppuccin-latte"),
    bundled!("catppuccin-mocha"),
    bundled!("dracula"),
    bundled!("everforest-dark"),
    bundled!("github-dark"),
    bundled!("github-light"),
    bundled!("gruvbox-dark"),
    bundled!("monokai"),
    bundled!("night-owl"),
    bundled!("nord"),
    bundled!("one-dark-pro"),
    bundled!("rose-pine"),
    bundled!("solarized-dark"),
    bundled!("solarized-light"),
    bundled!("tokyo-night"),
];

/// What every theme shares: Lattice's typeface and square corners.
const SHARED: &[(&str, &str)] = &[
    ("font-sans", "IBM Plex Mono, monospace"),
    ("font-mono", "IBM Plex Mono, monospace"),
    ("radius", "0px"),
];

/// A theme file in `themes/`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFile {
    name: String,
    appearance: Mode,
    /// Role name to CSS color: hex, `rgb()`, `oklch()`, another role as
    /// `var(--role)`, or a mix such as
    /// `color-mix(in srgb, var(--accent) 20%, transparent)`.
    colors: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    Light,
    Dark,
}

/// A bundled theme, ready to draw with.
#[derive(Debug)]
pub(crate) struct Theme {
    /// Its file name in `themes/`: what Settings saves.
    pub id: &'static str,
    /// What Settings shows.
    pub name: String,
    /// What the platform draws its own chrome in: the title bar, scroll
    /// bars, and native controls.
    pub appearance: Appearance,
    tokens: Tokens,
}

impl Theme {
    /// The theme `id` of file `json`, falling back on `base` (the default
    /// theme's tokens) for the roles it gets wrong, with what is wrong with
    /// it: names that are no role, and roles it leaves out or gives no
    /// color.
    fn parse(
        id: &'static str,
        json: &str,
        base: Option<&Tokens>,
    ) -> Result<(Self, Vec<String>), String> {
        let file: ThemeFile = serde_json::from_str(json).map_err(|error| error.to_string())?;

        let mut vars = serde_json::Map::new();
        for &(name, value) in SHARED {
            vars.insert(name.into(), value.into());
        }
        for &(name, role) in NSH_ALIASES {
            vars.insert(name.into(), format!("var(--{})", role.name()).into());
        }
        for (name, value) in &file.colors {
            vars.insert(name.clone(), value.clone().into());
        }
        let css = serde_json::json!({ "cssVars": { "theme": vars } }).to_string();
        // Exactly the tokens the file sets, so a role it got wrong is
        // missing rather than quietly NSH's default of the same name.
        let own = nsh_base::theme_overrides(&css).map_err(|error| error.to_string())?;

        let mut problems: Vec<String> = file
            .colors
            .keys()
            .filter(|name| !ROLES.iter().any(|role| role.name() == name.as_str()))
            .map(|name| format!("`{name}` is not a color role"))
            .collect();
        problems.extend(ROLES.iter().filter_map(|role| {
            let name = role.name();
            match (file.colors.get(name), own.color(name)) {
                (_, Some(_)) => None,
                (None, None) => Some(format!("`{name}` is missing")),
                (Some(value), None) => Some(format!("`{name}`: {value:?} is not a color")),
            }
        }));

        let base = base
            .cloned()
            .unwrap_or_else(|| nsh_base::Theme::default().into_tokens());
        let theme = Self {
            id,
            name: file.name,
            appearance: match file.appearance {
                Mode::Light => Appearance::Light,
                Mode::Dark => Appearance::Dark,
            },
            tokens: own.over(&base),
        };
        Ok((theme, problems))
    }
}

static THEMES: LazyLock<Vec<Theme>> = LazyLock::new(|| {
    let mut themes = Vec::<Theme>::with_capacity(BUNDLED.len());
    for &(id, json) in BUNDLED {
        let base = themes.first().map(|default| &default.tokens);
        match Theme::parse(id, json, base) {
            Ok((theme, problems)) => {
                for problem in problems {
                    log::warn!("Lattice: theme `{id}`: {problem}");
                }
                themes.push(theme);
            }
            Err(error) => log::error!("Lattice: skipping theme `{id}`: {error}"),
        }
    }
    assert!(!themes.is_empty(), "no bundled Lattice theme loads");
    themes
});

/// Every bundled theme, in Settings' order; the first is the default.
pub(crate) fn all() -> &'static [Theme] {
    &THEMES
}

/// The default theme's tokens, which the desktop launcher opens the window
/// with; the saved theme replaces them before the first frame.
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
pub fn tokens() -> Tokens {
    all()[0].tokens.clone()
}

fn index_of(id: &str) -> Option<usize> {
    all().iter().position(|theme| theme.id == id)
}

fn apply(theme: &Theme) {
    set_tokens(theme.tokens.clone());
    set_color_scheme(theme.appearance);
}

/// The theme the app is drawn with, which Settings changes.
#[derive(Clone, Copy)]
pub(crate) struct ActiveTheme(Signal<usize>);

impl ActiveTheme {
    /// Draws the app with the saved theme, or the default when none is
    /// saved or the saved one is no longer bundled.
    pub(crate) fn restore() -> Self {
        let saved = storage::load_settings().theme;
        let index = saved.as_deref().and_then(index_of).unwrap_or(0);
        apply(&all()[index]);
        Self(signal(index))
    }

    /// The theme now (tracked).
    pub(crate) fn get(self) -> &'static Theme {
        &all()[self.0.get()]
    }

    /// Switches to the theme `id`, and saves the choice.
    pub(crate) fn select(self, id: &str) {
        if let Some(index) = index_of(id) {
            self.switch(index);
        }
    }

    /// Switches `by` themes along Settings' order, wrapping around at
    /// either end (1: the next, -1: the previous), and saves the choice.
    pub(crate) fn step(self, by: isize) {
        let count = all().len() as isize;
        let index = (self.0.get_untracked() as isize + by).rem_euclid(count);
        self.switch(index as usize);
    }

    fn switch(self, index: usize) {
        if index == self.0.get_untracked() {
            return;
        }
        let theme = &all()[index];
        apply(theme);
        self.0.set(index);
        let mut settings = storage::load_settings();
        settings.theme = Some(theme.id.to_owned());
        if let Err(error) = storage::save_settings(&settings) {
            log::warn!("Lattice: unable to save settings: {error}");
        }
    }
}
