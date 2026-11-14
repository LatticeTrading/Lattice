//! What every widget shares: the [`Widget`] trait each one implements,
//! what its views get from its tab ([`TabContext`]), and the startup the
//! host runs for each of them once ([`build`]).
//!
//! Layout gives the host a tab whatever its kind ([`TabHandle`]); the
//! registry (`widgets.rs`) picks the widget by the tab's kind, and the
//! host turns the handle into that kind's [`TabContext`].

use std::{fmt::Debug, rc::Rc};

use nsh_core::prelude::*;
use serde::Serialize;
use serde_json::Value;

use super::{WidgetKind, WidgetSettings};
use crate::data::{DataServices, SessionScope, WidgetView};

/// One widget: how the picker shows it, its settings and session, and its
/// views. Each widget's `mod.rs` implements it on a unit struct, and the
/// registry lists that struct.
pub(crate) trait Widget: 'static {
    /// The old app's widget id, which tabs save and test ids use.
    const ID: &'static str;
    /// Its name, which a new tab of it is titled with.
    const TITLE: &'static str;
    /// One sentence on what it is for, which the picker shows.
    const DESCRIPTION: &'static str;
    /// Where the picker files it.
    const CATEGORY: Category;

    /// Its settings, saved in its tab's record: read leniently from the
    /// saved member (`From<Value>`), saved with `Serialize`, and left out
    /// while they equal `Default`.
    type Settings: KindSettings + Debug + Eq + Serialize + From<Value>;
    /// Its session state, which keeps its data while it is anywhere in the
    /// workspace.
    type Session: 'static;

    /// Makes its session state, under the session's owner, the first time
    /// a view of it asks. `settings` is the session's copy of its tab's
    /// settings, which the host makes and keeps equal to the tab's, so what
    /// the session does follows them even while no view shows it; a session
    /// that doesn't need them ignores it.
    fn session(
        scope: SessionScope,
        services: &DataServices,
        settings: Signal<Self::Settings>,
    ) -> Self::Session;

    /// Its view, from its session and its tab.
    fn view(session: &Rc<Self::Session>, tab: &TabContext<Self::Settings>) -> View;

    /// Its settings panel, which the host shows while the pane's gear has
    /// it open. The default is none.
    fn panel(_session: &Rc<Self::Session>, _tab: &TabContext<Self::Settings>) -> Option<Overlay> {
        None
    }
}

/// The picker's groups of widgets, in the order its strip shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Category {
    Charts,
    Market,
    OrderFlow,
    Trading,
    Portfolio,
    Tools,
}

impl Category {
    pub const ALL: &[Self] = &[
        Self::Charts,
        Self::Market,
        Self::OrderFlow,
        Self::Trading,
        Self::Portfolio,
        Self::Tools,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Charts => "Charts",
            Self::Market => "Market",
            Self::OrderFlow => "Order Flow",
            Self::Trading => "Trading",
            Self::Portfolio => "Portfolio",
            Self::Tools => "Tools",
        }
    }

    /// Whether any widget is filed under it; the picker leaves out those
    /// that would list nothing.
    pub fn has_widgets(self) -> bool {
        WidgetKind::ALL.iter().any(|kind| kind.category() == self)
    }
}

/// One kind's settings type, which its views are given ([`TabContext`]).
/// The registry implements it for each widget's settings.
pub(crate) trait KindSettings: Clone + Default + PartialEq + 'static {
    /// Its settings in `settings`, when they are its kind's.
    fn of(settings: &WidgetSettings) -> Option<&Self>;
    fn wrap(self) -> WidgetSettings;
}

/// A pane's settings panel: whether it is open, and the gear it opens
/// below.
#[derive(Clone, Copy)]
pub(crate) struct SettingsPanel {
    pub open: Signal<bool>,
    pub anchor: ElementRef,
}

/// Reads a tab's saved settings: calls its argument with them (tracked).
pub(crate) type ReadSettings = Rc<dyn Fn(&mut dyn FnMut(&WidgetSettings))>;

/// A widget's tab as layout gives it to the host, whatever its kind.
#[derive(Clone)]
pub(crate) struct TabHandle {
    /// The tab's kind, which never changes.
    pub kind: WidgetKind,
    /// Calls its argument with the settings saved in the tab record
    /// (tracked), without copying them: the host's memo of the kind's
    /// settings is the one copy.
    pub read: ReadSettings,
    /// Whether the tab is its pane's active one (its view shows).
    pub shown: Memo<bool>,
    /// Saves settings in the tab record and persists the workspace; does
    /// nothing when they are unchanged or the tab is gone.
    pub save: Rc<dyn Fn(&WidgetSettings)>,
    pub panel: SettingsPanel,
}

/// What a widget's views get from its tab, besides its session: the
/// settings saved in the tab record (`S`, its kind's settings type), and
/// its pane's settings panel.
pub(crate) struct TabContext<S: KindSettings> {
    /// The tab's saved settings.
    pub settings: Memo<S>,
    /// Whether the tab is its pane's active one (its view shows).
    pub shown: Memo<bool>,
    save: Rc<dyn Fn(&S)>,
    panel: SettingsPanel,
}

impl<S: KindSettings> Clone for TabContext<S> {
    fn clone(&self) -> Self {
        Self {
            settings: self.settings,
            shown: self.shown,
            save: self.save.clone(),
            panel: self.panel,
        }
    }
}

impl<S: KindSettings> TabContext<S> {
    /// The context of `tab`, whose kind's settings are `S`.
    fn new(tab: TabHandle) -> Self {
        let TabHandle {
            read,
            shown,
            save,
            panel,
            ..
        } = tab;
        Self {
            settings: computed(move || {
                let mut found = None;
                read(&mut |settings| found = S::of(settings).cloned());
                found.unwrap_or_default()
            }),
            shown,
            save: Rc::new(move |settings: &S| save(&settings.clone().wrap())),
            panel,
        }
    }

    /// Saves `settings` in the tab record (every breakpoint's copy of the
    /// tab) and persists the workspace.
    pub fn save(&self, settings: &S) {
        (self.save)(settings);
    }

    /// Whether the widget's settings panel is open: the pane's gear opened
    /// it while this tab is the active one (tracked).
    pub fn settings_open(&self) -> bool {
        self.shown.get() && self.panel.open.get()
    }

    pub fn close_settings(&self) {
        self.panel.open.set(false);
    }

    /// The gear the settings panel opens below.
    pub fn settings_anchor(&self) -> &ElementRef {
        &self.panel.anchor
    }
}

/// A `W` widget's session state as the host keeps it: the widget's own,
/// and the session's copy of its settings.
struct Hosted<W: Widget> {
    settings: Signal<W::Settings>,
    session: Rc<W::Session>,
}

/// A `W` widget's view of `widget`, with the startup every widget shares:
/// it finds the data services, gets (or makes) the widget's session and the
/// session's copy of the settings, keeps that copy equal to the tab's, and
/// shows the settings panel while it is open.
pub(super) fn build<W: Widget>(widget: &WidgetView, tab: TabHandle) -> View {
    let tab = TabContext::<W::Settings>::new(tab);
    let Some(services) = DataServices::try_get() else {
        log::error!("{}: no data services above the widget", W::TITLE);
        return view().flex_1();
    };
    let saved = tab.settings;
    // Made under the session's owner, so the copy lives as long as it.
    let hosted = widget.state(|scope| {
        let settings = signal(saved.get_untracked());
        Hosted::<W> {
            settings,
            session: Rc::new(W::session(scope, &services, settings)),
        }
    });
    let (copy, session) = (hosted.settings, hosted.session.clone());
    // The tab record is where the settings live; the session follows it.
    effect(move || {
        let next = saved.get();
        if copy.with_untracked(|current| *current != next) {
            copy.set(next);
        }
    });
    let panel = {
        let (session, tab) = (session.clone(), tab.clone());
        dynamic(move || {
            tab.settings_open()
                .then(|| W::panel(&session, &tab))
                .flatten()
        })
    };
    W::view(&session, &tab).child(panel)
}
