//! What a notification is, whoever posts it: what it says ([`Notice`]) and
//! how much it matters ([`Level`]), the one the center keeps
//! ([`Posted`]), and how the user has popups show ([`Prefs`],
//! [`Corner`]).

use std::rc::Rc;

use crate::storage::StoredNotifications;

/// How much a notification matters, which is the color of its edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Level {
    /// Something to know.
    #[default]
    Info,
    /// Something asked for came to be: an order traded.
    Success,
    /// Something to look at.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "nothing posts one yet; the first that does takes this out"
        )
    )]
    Warning,
    /// Something went wrong, or was lost.
    Danger,
}

/// Something to tell the user, as whatever posts it writes it: a title on
/// one line, and what there is to add to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Notice {
    pub level: Level,
    /// What happened, in a few words: it is cut where its line ends.
    pub title: String,
    /// The particulars, in sentences; `None` when the title says it all.
    pub body: Option<String>,
    /// Who says it, by the name the user knows it by (an exchange, a
    /// widget); `None` for Lattice itself.
    pub origin: Option<String>,
}

impl Notice {
    /// A notification that says `title` and nothing more.
    pub(crate) fn new(level: Level, title: impl Into<String>) -> Self {
        Self {
            level,
            title: title.into(),
            body: None,
            origin: None,
        }
    }

    /// With the particulars under its title.
    pub(crate) fn body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    /// With who says it beside its title.
    pub(crate) fn origin(mut self, origin: impl Into<String>) -> Self {
        self.origin = Some(origin.into());
        self
    }
}

/// A notification the center took: what was posted, when, and the number
/// that tells it from every other of this run. Cloning is cheap.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Posted {
    pub id: u64,
    /// When it was posted, in Unix milliseconds.
    pub at_ms: f64,
    pub notice: Rc<Notice>,
}

/// The corner of the window popups show in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Corner {
    #[default]
    BottomRight,
    BottomLeft,
    TopRight,
    TopLeft,
}

impl Corner {
    /// Every corner, in the order Settings offers them.
    pub(crate) const ALL: [Self; 4] = [
        Self::BottomRight,
        Self::BottomLeft,
        Self::TopRight,
        Self::TopLeft,
    ];

    /// What it is saved as.
    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::BottomRight => "bottom-right",
            Self::BottomLeft => "bottom-left",
            Self::TopRight => "top-right",
            Self::TopLeft => "top-left",
        }
    }

    /// What Settings calls it.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::BottomRight => "Bottom right",
            Self::BottomLeft => "Bottom left",
            Self::TopRight => "Top right",
            Self::TopLeft => "Top left",
        }
    }

    /// The corner saved as `id`.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|corner| corner.id() == id)
    }

    pub(crate) const fn is_top(self) -> bool {
        matches!(self, Self::TopRight | Self::TopLeft)
    }

    pub(crate) const fn is_left(self) -> bool {
        matches!(self, Self::BottomLeft | Self::TopLeft)
    }
}

/// How the user has notifications pop up: in the bottom right corner
/// unless they say otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Prefs {
    /// Whether a notification pops up as it arrives. The list keeps it
    /// either way.
    pub popups: bool,
    pub corner: Corner,
}

impl Default for Prefs {
    fn default() -> Self {
        Self::from_stored(&StoredNotifications::default())
    }
}

impl Prefs {
    /// What was saved, read leniently: a corner no longer offered is the
    /// default one.
    pub(super) fn from_stored(stored: &StoredNotifications) -> Self {
        Self {
            popups: stored.popups,
            corner: stored
                .corner
                .as_deref()
                .and_then(Corner::from_id)
                .unwrap_or_default(),
        }
    }

    pub(super) fn to_stored(self) -> StoredNotifications {
        StoredNotifications {
            popups: self.popups,
            corner: Some(self.corner.id().to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_save_their_corner_and_read_leniently() {
        assert_eq!(
            Prefs::default(),
            Prefs {
                popups: true,
                corner: Corner::BottomRight,
            }
        );
        for corner in Corner::ALL {
            let prefs = Prefs {
                popups: false,
                corner,
            };
            assert_eq!(Prefs::from_stored(&prefs.to_stored()), prefs);
        }
        // Settings saved before there were notifications, and a corner no
        // longer offered.
        let earlier: StoredNotifications = serde_json::from_str("{}").unwrap();
        assert_eq!(Prefs::from_stored(&earlier), Prefs::default());
        let gone: StoredNotifications =
            serde_json::from_str(r#"{"popups":false,"corner":"middle"}"#).unwrap();
        assert_eq!(
            Prefs::from_stored(&gone),
            Prefs {
                popups: false,
                corner: Corner::BottomRight,
            }
        );
    }
}
