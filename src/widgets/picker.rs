//! The widget picker (the old app's `WidgetPicker`), in the middle of the
//! window: a pane's `+` opens it to add a tab to that pane ("Add Widget
//! Tab"), the Everything Bar's New Widget to make a pane with the widget
//! picked ("Create Widget"). The app has one ([`Picker`]), so opening it
//! from another button moves it there. A strip of categories ("All", then
//! each category that has a widget) filters a list of widgets by name;
//! pressing one picks it and closes the picker.
//!
//! A widget's description shows beside its row, as the old app's popup did:
//! once the pointer rests on the row, or while the keyboard focus is on it
//! (an NSH hint overlay, which goes on the left when the right has no room).
//! Windows narrower than the old app's breakpoint, which have no room
//! beside the panel, show it inline under the name instead.

use std::time::Duration;

use nsh_core::{
    LineHeight,
    overlay::{Align, Placement},
    prelude::*,
};

use super::{Category, WidgetKind};
use crate::{layout, storage::StoredWorkspace, theme, ui};

const PANEL_WIDTH: Points = px(480.0);
const DESCRIPTION_WIDTH: Points = px(240.0);
/// The pointer's dwell on a row before its description shows, so sweeping
/// the list does not flash it.
const REVEAL_DELAY: Duration = Duration::from_millis(120);
/// Windows narrower than this show descriptions inline (the old app's
/// `max-width: 991px`).
const INLINE_BELOW: f32 = 992.0;

/// What the picked widget goes into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// A new pane on the active page (the Everything Bar's New Widget).
    NewPane,
    /// A new tab in a pane (its `+`); pane ids are unique per page only.
    Tab { page: usize, pane: usize },
}

impl Target {
    fn title(self) -> &'static str {
        match self {
            Self::NewPane => "Create Widget",
            Self::Tab { .. } => "Add Widget Tab",
        }
    }
}

/// The app's widget picker: what it is open for, if it is.
#[derive(Clone, Copy)]
pub(crate) struct Picker {
    target: Signal<Option<Target>>,
}

impl Picker {
    pub fn new() -> Self {
        Self {
            target: signal(None),
        }
    }

    /// Opens it for `target`; closes it when it is already open for it.
    pub fn toggle(self, target: Target) {
        self.target.update(|open| {
            *open = (*open != Some(target)).then_some(target);
        });
    }

    /// Whether it is open for `target` (tracked).
    pub fn is_open_for(self, target: Target) -> bool {
        self.target.get() == Some(target)
    }

    fn close(self) {
        self.target.set(None);
    }

    /// The picker while it is open, adding what is picked to `workspace`.
    /// It closes itself when its pane goes or another page is shown.
    pub fn view(self, workspace: Signal<StoredWorkspace>) -> impl IntoElement {
        effect(move || {
            let Some(Target::Tab { page, pane }) = self.target.get() else {
                return;
            };
            let there = workspace
                .with(|state| state.active_page_id == page && layout::has_pane(state, pane));
            if !there {
                self.close();
            }
        });
        dynamic(move || {
            let target = self.target.get()?;
            let pick = move |kind: WidgetKind| {
                batch(|| {
                    self.close();
                    workspace.update(|state| {
                        match target {
                            Target::NewPane => layout::spawn(state, kind),
                            Target::Tab { page, pane } => {
                                if state.active_page_id != page {
                                    return;
                                }
                                layout::add_tab(state, pane, kind);
                            }
                        }
                        layout::persist(state);
                    });
                });
            };
            Some(panel(target.title(), pick, move || self.close()))
        })
    }
}

/// The picker titled `title`. Picking a widget calls `on_pick`; closing it,
/// `on_close`.
fn panel(
    title: &str,
    on_pick: impl Fn(WidgetKind) + Clone + 'static,
    on_close: impl Fn() + Clone + 'static,
) -> Overlay {
    // `None` is "All". Every opening starts there.
    let filter = signal(None::<Category>);
    let inline = computed(|| window().width.0 < INLINE_BELOW);
    let body = view()
        .test_id("widget-picker")
        .flex_col()
        .text_size(px(11.0))
        .text_color(theme::TEXT)
        .child(categories(filter))
        .child(dynamic(move || list(filter.get(), inline, on_pick.clone())));
    ui::centered_panel(PANEL_WIDTH, title, on_close, body)
}

/// "All", then every category with a widget in it.
fn categories(filter: Signal<Option<Category>>) -> View {
    let all = std::iter::once((None, "All")).chain(
        Category::ALL
            .iter()
            .filter(|category| category.has_widgets())
            .map(|category| (Some(*category), category.name())),
    );
    let mut strip = view()
        .flex_row()
        .flex_none()
        .bg(theme::SURFACE)
        .border_b_1()
        .border_color(theme::BORDER)
        .role(AccessibilityRole::Tablist)
        .label("Widget categories");
    for (category, name) in all {
        strip = strip.child(category_button(category, name, filter));
    }
    strip
}

fn category_button(
    category: Option<Category>,
    name: &'static str,
    filter: Signal<Option<Category>>,
) -> View {
    let active = move || filter.get() == category;
    view()
        .relative()
        .h(px(34.0))
        .flex()
        .items_center()
        .px(px(12.0))
        .text_size(px(10.0))
        .text_color(move || {
            if active() {
                theme::TEXT.get()
            } else {
                theme::TEXT_MUTED.get()
            }
        })
        .bg(move || {
            if active() {
                theme::BACKGROUND.get()
            } else {
                theme::SURFACE.get()
            }
        })
        .border_r_1()
        .border_color(theme::BORDER)
        .role(AccessibilityRole::Tab)
        .label(name)
        .accessibility_state(move || AccessibilityState {
            selected: active(),
            ..Default::default()
        })
        .hover(|style| style.bg(theme::BACKGROUND).text_color(theme::TEXT))
        .active(|style| style.bg(theme::BORDER))
        .on_press(move || filter.set(category))
        .child(text(name))
        .child(
            view()
                .absolute()
                .left(px(0.0))
                .right(px(0.0))
                .bottom(px(0.0))
                .h(px(2.0))
                .bg(theme::ACCENT)
                .style_when(move || !active(), |style| style.hidden()),
        )
}

/// The widgets of `category` (every widget for `None`).
fn list(
    category: Option<Category>,
    inline: Memo<bool>,
    on_pick: impl Fn(WidgetKind) + Clone + 'static,
) -> View {
    let kinds: Vec<WidgetKind> = WidgetKind::ALL
        .iter()
        .copied()
        .filter(|kind| category.is_none_or(|category| kind.category() == category))
        .collect();
    let last = kinds.len().saturating_sub(1);
    let mut list = view()
        .flex_col()
        .role(AccessibilityRole::List)
        .label(format!(
            "{} widgets",
            category.map_or("All", Category::name)
        ));
    for (index, kind) in kinds.into_iter().enumerate() {
        list = list.child(row(kind, index == last, inline, on_pick.clone()));
    }
    list
}

/// One widget: its name, a bar at its left edge while hovered, and its
/// description beside it (or under the name in a narrow window).
fn row(
    kind: WidgetKind,
    last: bool,
    inline: Memo<bool>,
    on_pick: impl Fn(WidgetKind) + 'static,
) -> View {
    let anchor = element_ref();
    let hovered = signal(false);
    view()
        .element_ref(&anchor)
        .relative()
        .w_full()
        .min_h(move || px(if inline.get() { 44.0 } else { 36.0 }))
        .flex_col()
        .justify_center()
        .gap(px(2.0))
        .px(px(12.0))
        .py(move || px(if inline.get() { 6.0 } else { 0.0 }))
        .map(|row| {
            if last {
                row
            } else {
                row.border_b_1().border_color(theme::BORDER)
            }
        })
        .role(AccessibilityRole::Button)
        .label(format!("Add {}", kind.title()))
        .hint(kind.description())
        .test_id(&format!("widget-picker-{}", kind.id()))
        .hover(|style| style.bg(theme::SURFACE))
        .active(|style| style.bg(theme::BORDER))
        .on_mouse_enter(move |_: &MouseEvent| hovered.set(true))
        .on_mouse_leave(move |_: &MouseEvent| hovered.set(false))
        .on_press(move || on_pick(kind))
        // Not every theme's hover fill stands out from its panel; the bar
        // does.
        .child(
            view()
                .absolute()
                .left(px(0.0))
                .top(px(0.0))
                .bottom(px(0.0))
                .w(px(2.0))
                .bg(theme::ACCENT)
                .style_when(move || !hovered.get(), |style| style.hidden()),
        )
        .child(
            text(kind.title())
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM),
        )
        .child(dynamic(move || {
            inline.get().then(|| {
                text(kind.description())
                    .text_size(px(10.0))
                    .line_height(LineHeight::Multiple(1.35))
            })
        }))
        .child(dynamic(move || {
            (!inline.get()).then(|| description(kind, &anchor))
        }))
}

/// The description beside `row`, top edges level, while the pointer rests
/// on it or the keyboard focus is on it.
fn description(kind: WidgetKind, row: &ElementRef) -> Overlay {
    overlay()
        .anchor(row)
        .placement(Placement::Right)
        .align(Align::Start)
        .offset(px(8.0))
        .margin(px(8.0))
        .hint(REVEAL_DELAY)
        .pointer_events(PointerEvents::None)
        .child(
            view()
                .w(DESCRIPTION_WIDTH)
                .px(px(10.0))
                .py(px(8.0))
                .bg(theme::BACKGROUND)
                .border_1()
                .border_color(theme::BORDER)
                .text_size(px(11.0))
                .text_color(theme::TEXT)
                .child(text(kind.description()).line_height(LineHeight::Multiple(1.45))),
        )
}
