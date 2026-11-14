//! Color linking: the colors a widget's market can be linked to, and the
//! one the Everything Bar's search sends its picks to (the old app's
//! `linkColors.ts` and `useColorLinking`).
//!
//! A widget that shows one market (the Chart) is linked to a color, or to
//! none: blue until its pane header's link button says otherwise, as the
//! old app's charts were. The Everything Bar has a color too
//! ([`ActiveLink`]), which its own link button changes. Picking a market in
//! the bar's search shows it on every widget linked to the bar's color, on
//! every page ([`crate::layout::link_market`]), and on no other.
//!
//! The pick is written into those widgets' saved settings, so a linked
//! widget's settings panel names the market it shows and a restart keeps
//! it. (The old app kept one market per color beside the widgets'
//! settings, which a widget's own market setting silently overrode.)

use nsh_core::prelude::*;

use crate::{
    storage,
    theme::{self, Role},
};

/// A link color: one group of widgets that take the same market.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum LinkColor {
    Green,
    Red,
    Blue,
    Yellow,
    Orange,
}

impl LinkColor {
    /// Every color, in the old app's order.
    pub const ALL: [Self; 5] = [
        Self::Green,
        Self::Red,
        Self::Blue,
        Self::Yellow,
        Self::Orange,
    ];

    /// What a widget is linked to until it is told otherwise, and where
    /// the Everything Bar's picks go until another color is chosen (the
    /// old app's `DEFAULT_LINK_COLOR`).
    pub const DEFAULT: Self = Self::Blue;

    /// Its place in [`Self::ALL`].
    pub const fn index(self) -> usize {
        self as usize
    }

    /// What settings save for it (the old app's names).
    pub const fn id(self) -> &'static str {
        match self {
            Self::Green => "green",
            Self::Red => "red",
            Self::Blue => "blue",
            Self::Yellow => "yellow",
            Self::Orange => "orange",
        }
    }

    /// The color a saved id names.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|color| color.id() == id)
    }

    /// Its name.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Green => "Green",
            Self::Red => "Red",
            Self::Blue => "Blue",
            Self::Yellow => "Yellow",
            Self::Orange => "Orange",
        }
    }

    /// The role it is drawn with, which each theme takes from its own
    /// palette.
    pub const fn role(self) -> Role {
        match self {
            Self::Green => theme::LINK_GREEN,
            Self::Red => theme::LINK_RED,
            Self::Blue => theme::LINK_BLUE,
            Self::Yellow => theme::LINK_YELLOW,
            Self::Orange => theme::LINK_ORANGE,
        }
    }
}

/// The Everything Bar's link color: where its search sends a pick.
#[derive(Clone, Copy)]
pub(crate) struct ActiveLink(Signal<LinkColor>);

impl ActiveLink {
    /// The saved color, or the default when none (or one no longer
    /// offered) is saved.
    pub(crate) fn restore() -> Self {
        let saved = storage::load_settings().link_color;
        Self(signal(
            saved
                .as_deref()
                .and_then(LinkColor::from_id)
                .unwrap_or(LinkColor::DEFAULT),
        ))
    }

    /// The color now (tracked).
    pub(crate) fn get(self) -> LinkColor {
        self.0.get()
    }

    /// The color now, without subscribing.
    pub(crate) fn get_untracked(self) -> LinkColor {
        self.0.get_untracked()
    }

    /// Sends the bar's picks to `color` from now on, and saves the choice.
    pub(crate) fn select(self, color: LinkColor) {
        if color == self.0.get_untracked() {
            return;
        }
        self.0.set(color);
        let mut settings = storage::load_settings();
        settings.link_color = Some(color.id().to_owned());
        if let Err(error) = storage::save_settings(&settings) {
            log::warn!("Lattice: unable to save settings: {error}");
        }
    }
}
