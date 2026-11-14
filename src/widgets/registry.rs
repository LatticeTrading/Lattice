//! The macro behind the registry (`widgets.rs`). From one line per widget,
//! `module::Type` naming its [`Widget`](super::host::Widget) impl, in the
//! order the picker lists them, it makes:
//!
//! - [`WidgetKind`](super::WidgetKind): the catalog of widgets, a variant
//!   per widget named as its type, with the widget's metadata;
//! - [`WidgetSettings`](super::WidgetSettings): a tab's kind and settings,
//!   and how they are saved;
//! - [`KindSettings`](super::host::KindSettings) for each widget's
//!   settings;
//! - [`view`](super::view): a tab's view, by its kind.

macro_rules! registry {
    ($($module:ident :: $kind:ident),+ $(,)?) => {
        /// A widget a tab can be: its name, what it is for, and where the
        /// picker files it (the old app's `widgetRegistry` and picker
        /// catalog).
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub(crate) enum WidgetKind {
            $($kind,)+
        }

        impl WidgetKind {
            /// Every widget, in the order the picker lists them (the
            /// registry's, which is the old app's).
            pub const ALL: &[Self] = &[$(Self::$kind,)+];

            /// The old app's widget id, which tabs save and test ids use.
            pub const fn id(self) -> &'static str {
                match self {
                    $(Self::$kind => <$module::$kind as $crate::widgets::host::Widget>::ID,)+
                }
            }

            /// The widget a saved id names.
            pub fn from_id(id: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|kind| kind.id() == id)
            }

            /// Its name, which a new tab of it is titled with.
            pub const fn title(self) -> &'static str {
                match self {
                    $(Self::$kind => <$module::$kind as $crate::widgets::host::Widget>::TITLE,)+
                }
            }

            /// One sentence on what it is for, which the picker shows.
            pub const fn description(self) -> &'static str {
                match self {
                    $(
                        Self::$kind => {
                            <$module::$kind as $crate::widgets::host::Widget>::DESCRIPTION
                        }
                    )+
                }
            }

            pub const fn category(self) -> $crate::widgets::host::Category {
                match self {
                    $(
                        Self::$kind => {
                            <$module::$kind as $crate::widgets::host::Widget>::CATEGORY
                        }
                    )+
                }
            }
        }

        /// A tab's widget and its settings: the kind is the variant, so a
        /// tab can never hold another kind's settings.
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub(crate) enum WidgetSettings {
            $($kind(<$module::$kind as $crate::widgets::host::Widget>::Settings),)+
        }

        impl WidgetSettings {
            /// A new widget of `kind`, with its default settings.
            pub fn new(kind: WidgetKind) -> Self {
                match kind {
                    $(WidgetKind::$kind => Self::$kind(Default::default()),)+
                }
            }

            pub fn kind(&self) -> WidgetKind {
                match self {
                    $(Self::$kind(_) => WidgetKind::$kind,)+
                }
            }

            /// Whether they are the kind's defaults (the tab saves none).
            pub fn is_default(&self) -> bool {
                match self {
                    $(Self::$kind(settings) => *settings == Default::default(),)+
                }
            }

            /// `kind`'s settings read from a saved member, leniently (a
            /// damaged member reads as defaults).
            pub fn read(kind: WidgetKind, saved: ::serde_json::Value) -> Self {
                match kind {
                    $(WidgetKind::$kind => Self::$kind(saved.into()),)+
                }
            }

            /// The saved member.
            pub fn to_value(&self) -> ::serde_json::Value {
                let value = match self {
                    $(Self::$kind(settings) => ::serde_json::to_value(settings),)+
                };
                value.unwrap_or(::serde_json::Value::Null)
            }
        }

        $(
            impl $crate::widgets::host::KindSettings
                for <$module::$kind as $crate::widgets::host::Widget>::Settings
            {
                fn of(settings: &WidgetSettings) -> Option<&Self> {
                    match settings {
                        WidgetSettings::$kind(settings) => Some(settings),
                        #[allow(unreachable_patterns, reason = "while there is one widget")]
                        _ => None,
                    }
                }

                fn wrap(self) -> WidgetSettings {
                    WidgetSettings::$kind(self)
                }
            }
        )+

        /// A tab's view: its widget's, by the tab's kind, with the startup
        /// every widget shares ([`host::build`]).
        pub(crate) fn view(
            widget: &$crate::data::WidgetView,
            tab: $crate::widgets::host::TabHandle,
        ) -> ::nsh_core::prelude::View {
            match tab.kind() {
                $(WidgetKind::$kind => $crate::widgets::host::build::<$module::$kind>(widget, tab),)+
            }
        }
    };
}

pub(super) use registry;
