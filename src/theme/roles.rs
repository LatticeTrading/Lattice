//! The color roles: the named colors the UI draws with, which every theme
//! file in `themes/` gives a value.
//!
//! A role says what a color is for, never what it looks like (`TEXT_MUTED`,
//! not `GRAY`), so each theme is free to pick its own. To add one, add it
//! to the list below with what it is for, then give it a color in every
//! theme file; `cargo test` names any theme that misses it.

use std::rc::Rc;

use nsh_core::{
    element::{IntoReactive, Reactive},
    prelude::*,
};

/// A themed color: a role, at an opacity. Every color style takes one
/// (`.bg(theme::SURFACE)`, `.text_color(theme::ACCENT.opacity(0.5))`) and
/// reads it again whenever the theme changes, so the open UI repaints in
/// place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Role {
    name: &'static str,
    opacity: f32,
}

impl Role {
    const fn named(name: &'static str) -> Self {
        Self { name, opacity: 1.0 }
    }

    /// The key theme files give it a color under; also its NSH token's
    /// name, so it is a Tailwind color too (`bg-surface`).
    pub(crate) const fn name(self) -> &'static str {
        self.name
    }

    /// This color with its alpha multiplied by `opacity`.
    pub(crate) const fn opacity(self, opacity: f32) -> Self {
        Self {
            opacity: self.opacity * opacity,
            ..self
        }
    }

    /// Its color in the theme now. Reading it subscribes: what read it runs
    /// again when the theme changes.
    pub(crate) fn get(self) -> AdaptiveColor {
        nsh_core::element::tokens()
            .color(self.name)
            .unwrap_or(AdaptiveColor::TRANSPARENT)
            .opacity(self.opacity)
    }
}

impl IntoReactive<AdaptiveColor> for Role {
    fn into_reactive(self) -> Reactive<AdaptiveColor> {
        Reactive::Bound(Rc::new(move || self.get()))
    }
}

/// For props that take one color rather than one per appearance (a text
/// field's caret, placeholder, and selection). A switch applies the tokens
/// and the appearance as two steps, so this may resolve against the old
/// appearance for a moment; that is harmless because themes give both
/// appearances the same color.
impl IntoReactive<Hsla> for Role {
    fn into_reactive(self) -> Reactive<Hsla> {
        Reactive::Bound(Rc::new(move || nsh_core::element::color(self.get())))
    }
}

macro_rules! roles {
    ($( $(#[doc = $doc:literal])* $role:ident = $name:literal, )*) => {
        $( $(#[doc = $doc])* pub(crate) const $role: Role = Role::named($name); )*

        /// Every role, in the order theme files list them.
        pub(crate) const ROLES: &[Role] = &[$($role),*];
    };
}

roles! {
    // Surfaces, from the back of the window forward.
    /// The window behind everything, pane bodies, text fields, the selected
    /// tab, and floating panels' title bars.
    BACKGROUND = "background",
    /// The Everything Bar, pane headers, and inactive tabs; also what a
    /// hovered control fills with.
    SURFACE = "surface",
    /// Floating panels: the Layout Manager, Settings, and their dropdowns.
    PANEL = "panel",
    /// Borders and dividers; also what a pressed control fills with.
    BORDER = "border",

    // Text and icons.
    /// Primary text and icons.
    TEXT = "text",
    /// Secondary text: labels, inactive tabs, and Everything Bar controls.
    TEXT_MUTED = "text-muted",
    /// The quietest text: placeholders, and close, drag, and resize glyphs.
    TEXT_FAINT = "text-faint",

    // Meaning.
    /// What is selected or active: the current page, the caret, selected
    /// text, the chosen dropdown item.
    ACCENT = "accent",
    /// Errors and destructive actions.
    DANGER = "danger",
    /// Confirmations.
    SUCCESS = "success",
}

/// NSH's standard token names (shadcn/ui's) and the role each is drawn
/// from, so the pieces NSH draws itself (Linux's menus, selection
/// highlights) match the theme too. `background`, `border`, and `accent`
/// are roles of the same name already. Theme files never set these.
pub(super) const NSH_ALIASES: &[(&str, Role)] = &[
    ("foreground", TEXT),
    ("card", SURFACE),
    ("card-foreground", TEXT),
    ("popover", PANEL),
    ("popover-foreground", TEXT),
    ("primary", ACCENT),
    // Whatever a theme's accent is, its background reads on it.
    ("primary-foreground", BACKGROUND),
    ("accent-foreground", BACKGROUND),
    ("ring", ACCENT),
    ("muted-foreground", TEXT_MUTED),
    ("destructive", DANGER),
    ("input", BORDER),
];
