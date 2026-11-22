//! Notifications: the one place anything in the app tells the user that
//! something happened, whatever they are looking at.
//!
//! - [`Notice`] is what is told: how much it matters ([`Level`]), a
//!   title, and optionally the particulars and who says it. It names no
//!   exchange, widget or kind of event: whatever posts one writes it in
//!   words.
//! - [`Notifications`] is the center the shell makes and provides as
//!   context. [`post`](Notifications::post) is all a producer calls. The
//!   center keeps the newest [`KEPT`] for the Everything Bar's bell, counts
//!   those not yet seen there, and holds the few that are popped up in a
//!   corner of the window now ([`SHOWN`] at most). Nothing is kept between
//!   runs.
//! - [`Prefs`] is how the user has popups show (on or off, and the
//!   [`Corner`]), which Settings changes and which is saved with the other
//!   preferences.
//!
//! Nothing here draws: the bell, its list and the popups are the shell's
//! (`crate::shell::notifications`, `everything_bar/bell.rs`), and they
//! show whatever is posted the same way.
//!
//! **Posting from a new place** (a widget's session, another exchange's
//! events, a price that was crossed): take the center where the thing is
//! made, under the shell (`Notifications::try_get()`; `None` for a
//! component rendered on its own, which then has nobody to tell), keep the
//! copy, and `post` when it happens, from any callback. Post from the one
//! place that knows of the event once (a service, a session), never from a
//! view, or two views of it tell it twice. Nothing else changes: no list
//! of kinds to extend, nothing to register.

mod model;

pub(crate) use model::{Corner, Level, Notice, Posted, Prefs};

use std::rc::Rc;

use nsh_core::{prelude::*, time::unix_ms};

use crate::storage;

/// How many notifications the bell's list keeps: the newest.
pub(crate) const KEPT: usize = 100;
/// How many popups show at once: the newest. One that arrives while that
/// many show takes the oldest's place.
pub(crate) const SHOWN: usize = 3;

/// The app's notifications. Copying is cheap: every copy is the same
/// center.
#[derive(Clone, Copy)]
pub(crate) struct Notifications {
    /// What was posted, newest first.
    history: Signal<Vec<Posted>>,
    /// What is popped up now, oldest first.
    popups: Signal<Vec<Posted>>,
    /// How many were posted since the list was last looked at.
    unread: Signal<usize>,
    prefs: Signal<Prefs>,
    /// The last id given. Nothing reads it but the next one: it is a
    /// signal so that the center stays `Copy`.
    last: Signal<u64>,
}

impl Notifications {
    /// Makes the center under the current owner (the shell), with the
    /// saved preferences, and provides it to everything made under it from
    /// now on.
    pub(crate) fn provide() -> Self {
        let center = Self {
            history: signal(Vec::new()),
            popups: signal(Vec::new()),
            unread: signal(0),
            prefs: signal(Prefs::from_stored(&storage::load_settings().notifications)),
            last: signal(0),
        };
        provide_context(center);
        center
    }

    /// The center the shell provided, or `None` outside the shell (a
    /// component rendered on its own, as some tests do).
    pub(crate) fn try_get() -> Option<Self> {
        use_context::<Self>()
    }

    /// Tells the user `notice`: it joins the bell's list, and pops up
    /// unless the user turned popups off.
    pub(crate) fn post(self, notice: Notice) {
        let posted = self.stamp(notice);
        batch(|| {
            self.history.update(|history| {
                history.insert(0, posted.clone());
                history.truncate(KEPT);
            });
            // No more are unread than the list holds.
            self.unread
                .update(|unread| *unread = (*unread + 1).min(KEPT));
            self.pop(posted);
        });
    }

    /// Shows where popups appear, with one that is not kept: what Settings
    /// answers a change of them with. Nothing when one shows there already,
    /// which says as much.
    pub(crate) fn preview(self) {
        if self.popups.with_untracked(Vec::is_empty) {
            self.pop(self.stamp(Notice::new(Level::Info, "Notifications show here")));
        }
    }

    /// What was posted, newest first (tracked).
    pub(crate) fn history(self) -> Vec<Posted> {
        self.history.get()
    }

    /// Whether nothing was posted, or all of it was cleared (tracked).
    pub(crate) fn is_empty(self) -> bool {
        self.history.with(Vec::is_empty)
    }

    /// What is popped up now, oldest first (tracked).
    pub(crate) fn popups(self) -> Vec<Posted> {
        self.popups.get()
    }

    /// Whether anything is popped up (tracked).
    pub(crate) fn has_popups(self) -> bool {
        !self.popups.with(Vec::is_empty)
    }

    /// How many were posted since the list was last looked at (tracked).
    pub(crate) fn unread(self) -> usize {
        self.unread.get()
    }

    /// The list is being looked at: nothing in it is unread.
    pub(crate) fn mark_read(self) {
        self.unread.set(0);
    }

    /// Takes the popup `id` down. The list keeps it.
    pub(crate) fn dismiss(self, id: u64) {
        if self
            .popups
            .with_untracked(|popups| popups.iter().any(|popup| popup.id == id))
        {
            self.popups
                .update(|popups| popups.retain(|popup| popup.id != id));
        }
    }

    /// Forgets everything posted so far, popped up or listed.
    pub(crate) fn clear(self) {
        batch(|| {
            self.history.set(Vec::new());
            self.popups.set(Vec::new());
            self.unread.set(0);
        });
    }

    /// How the user has popups show (tracked).
    pub(crate) fn prefs(self) -> Prefs {
        self.prefs.get()
    }

    /// Changes how popups show and saves it. Turned off, those that show
    /// go.
    pub(crate) fn update(self, change: impl FnOnce(&mut Prefs)) {
        let before = self.prefs.get_untracked();
        let mut prefs = before;
        change(&mut prefs);
        if prefs == before {
            return;
        }
        batch(|| {
            self.prefs.set(prefs);
            if !prefs.popups {
                self.popups.set(Vec::new());
            }
        });
        let mut settings = storage::load_settings();
        settings.notifications = prefs.to_stored();
        if let Err(error) = storage::save_settings(&settings) {
            log::warn!("Lattice: unable to save settings: {error}");
        }
    }

    /// `notice` as the center keeps one: numbered, and timed now.
    fn stamp(self, notice: Notice) -> Posted {
        let id = self.last.get_untracked() + 1;
        self.last.set(id);
        Posted {
            id,
            at_ms: unix_ms(),
            notice: Rc::new(notice),
        }
    }

    /// Pops `posted` up, in the place of the oldest when [`SHOWN`] show
    /// already; nothing while popups are off.
    fn pop(self, posted: Posted) {
        if !self.prefs.get_untracked().popups {
            return;
        }
        self.popups.update(|popups| {
            popups.push(posted);
            let over = popups.len().saturating_sub(SHOWN);
            popups.drain(..over);
        });
    }
}
