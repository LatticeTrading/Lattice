//! The Watchlist's bar: a tab per list, then `+ New List`.
//!
//! - A tab is its name (a press shows its list) and a ▾ cell after it, as
//!   the pane's tabs have their ×. The ▾ opens a menu hanging from it:
//!   Rename, and Remove (which asks again in place, and is disabled for the
//!   last list left).
//! - A tab dragged past a slop by its name is picked up, as the pane's tabs
//!   are: it dims, a bar marks the gap among the tabs it would go to (before
//!   the first tab whose middle is past the pointer, else after the last),
//!   and dropping it there moves its list. Nothing is saved before the
//!   drop.
//! - Rename turns the name into a field in the tab, which Enter or leaving
//!   it saves; a blank name keeps the old one.
//! - `+ New List` adds an empty list named for how many there are with it
//!   (`List 3` when there were two), shows it, and opens its name for
//!   editing.
//! - The tabs own their right edge and scroll sideways when the pane is
//!   narrow; the shown one is filled with the body's `BACKGROUND`.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use nsh_core::{
    AccessibilityActionDef, PanActivation,
    overlay::{Align, Placement},
    prelude::*,
};

use super::{NAME_LENGTH, Refused, WatchlistSettings};
use crate::{theme, ui, widgets::TabContext};

/// The bar's height, without its line.
const BAR_HEIGHT: f32 = 32.0;
/// A tab's name's leading padding, and the gap before its ▾.
const TAB_PADDING: f32 = 12.0;
const TAB_GAP: f32 = 4.0;
/// The ▾ cell's width.
const MENU_CELL: f32 = 24.0;
/// The name's field while it is edited.
const RENAME_WIDTH: f32 = 160.0;
/// The menu's narrowest, and a row's height.
const MENU_WIDTH: f32 = 140.0;
const MENU_ROW: f32 = 30.0;
/// How far a tab is dragged before it is picked up rather than pressed.
const SLOP: f32 = 6.0;
/// The bar that marks the gap a dragged tab would go to.
const GAP_BAR: f32 = 3.0;

/// A tab being dragged among the others.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Drag {
    /// The list dragged.
    from: usize,
    /// The gap among the tabs it would go to if dropped now: 0 is before
    /// the first, the count after the last.
    gap: usize,
}

impl Drag {
    /// Where the list goes when dropped at its gap; `None` when that is
    /// where it is.
    fn to(self) -> Option<usize> {
        let to = if self.gap > self.from {
            self.gap - 1
        } else {
            self.gap
        };
        (to != self.from).then_some(to)
    }
}

/// The gap among tabs as wide as `widths`, side by side from 0, that `x`
/// is at: before the first whose middle is past it, else after the last.
fn gap_at(widths: &[f32], x: f32) -> usize {
    let mut left = 0.0;
    for (at, width) in widths.iter().enumerate() {
        if x < left + width / 2.0 {
            return at;
        }
        left += width;
    }
    widths.len()
}

/// What the bar's parts share: the tab, the list being renamed, the list
/// whose menu is open, and whether its Remove asked again.
#[derive(Clone)]
struct Bar {
    tab: TabContext<WatchlistSettings>,
    renaming: Signal<Option<usize>>,
    menu: Signal<Option<usize>>,
    confirming: Signal<bool>,
    /// Why the last pick was refused, which a change of list forgets.
    refusal: Signal<Option<Refused>>,
    /// The tab being dragged, once picked up.
    drag: Signal<Option<Drag>>,
    /// Each tab's width as laid out, in order.
    widths: Rc<RefCell<Vec<f32>>>,
}

impl Bar {
    fn save(&self, change: impl FnOnce(WatchlistSettings) -> Option<WatchlistSettings>) {
        if let Some(next) = change(self.tab.settings.get_untracked()) {
            self.tab.save(&next);
        }
    }

    fn close_menu(&self) {
        batch(|| {
            self.menu.set(None);
            self.confirming.set(false);
        });
    }
}

/// The bar of `tab`'s lists.
pub(super) fn bar(tab: TabContext<WatchlistSettings>, refusal: Signal<Option<Refused>>) -> View {
    let bar = Bar {
        tab,
        renaming: signal(None),
        menu: signal(None),
        confirming: signal(false),
        refusal,
        drag: signal(None),
        widths: Rc::default(),
    };
    let saved = bar.tab.settings;
    let shape = computed(move || {
        saved.with(|settings| {
            let names: Vec<String> = settings.lists().into_iter().map(|list| list.name).collect();
            (names, settings.active())
        })
    });
    // The menu of a list that is gone closes.
    {
        let bar = bar.clone();
        effect(move || {
            let count = shape.with(|(names, _)| names.len());
            if bar.menu.get().is_some_and(|at| at >= count) {
                bar.close_menu();
            }
        });
    }
    let tabs = {
        let bar = bar.clone();
        dynamic(move || {
            let (names, active) = shape.get();
            let renaming = bar.renaming.get();
            let count = names.len();
            bar.widths.borrow_mut().resize(count, 0.0);
            names
                .into_iter()
                .enumerate()
                .map(|(at, name)| {
                    list_tab(&bar, at, name, at == active, count, renaming == Some(at))
                })
                .collect::<Vec<_>>()
        })
    };
    view()
        .h(px(BAR_HEIGHT + 1.0))
        .w_full()
        .flex_none()
        .flex_row()
        .bg(theme::SURFACE)
        .border_b_1()
        .border_color(theme::BORDER)
        .test_id("watchlist-tabs")
        .child(
            scroll()
                .horizontal()
                .flex_1()
                .min_w(px(0.0))
                .h(px(BAR_HEIGHT))
                .label("Lists")
                .child(
                    view()
                        .h(px(BAR_HEIGHT))
                        .flex_row()
                        .child(tabs)
                        .child(new_list(&bar)),
                ),
        )
}

/// The tab of the list at `at`: its name (or its field while renamed),
/// then its ▾.
fn list_tab(bar: &Bar, at: usize, name: String, shown: bool, count: usize, renaming: bool) -> View {
    let label = if renaming {
        rename_field(bar, at, name.clone())
    } else {
        let select = bar.clone();
        let label = view()
            .h_full()
            .flex()
            .items_center()
            .pl(px(TAB_PADDING))
            .pr(px(TAB_GAP))
            .role(AccessibilityRole::Tab)
            .label(format!("List {name}"))
            .accessibility_state(move || AccessibilityState {
                selected: shown,
                ..Default::default()
            })
            .test_id(&format!("watchlist-tab-{at}"))
            .child(text(name.clone()).number_of_lines(1));
        draggable(bar, at, label, move || {
            if !shown {
                select.refusal.set(None);
                select.save(|settings| Some(settings.select(at)));
            }
        })
        .into_element()
    };
    let (drag, widths) = (bar.drag, bar.widths.clone());
    let held = move || drag.with(|drag| drag.is_some_and(|drag| drag.from == at));
    view()
        .h_full()
        .flex_none()
        .relative()
        .flex_row()
        .items_center()
        .on_layout(move |bounds: &Bounds<Points>| {
            if let Some(width) = widths.borrow_mut().get_mut(at) {
                *width = bounds.size.width.0;
            }
        })
        .opacity(move || if held() { 0.4_f32 } else { 1.0 })
        .text_size(px(11.0))
        .text_color(if shown {
            theme::TEXT
        } else {
            theme::TEXT_MUTED
        })
        .bg(if shown {
            theme::BACKGROUND
        } else {
            theme::SURFACE
        })
        .hover(move |s| if shown { s } else { s.bg(theme::PANEL) })
        .border_r_1()
        .border_color(theme::BORDER)
        .child(label)
        .child(menu_cell(bar, at, name, count))
        .child(gap_bar(drag, at, false))
        .children((at + 1 == count).then(|| gap_bar(drag, at + 1, true)))
}

/// `label`, the name of the list at `at`, made what drags its tab: past
/// the slop it picks the tab up, and the drop moves its list. A press that
/// ends a drag is not one of the tab's: `on_press` hears the others.
fn draggable(bar: &Bar, at: usize, label: View, on_press: impl Fn() + 'static) -> View {
    let (drag, widths) = (bar.drag, bar.widths.clone());
    let dragged = Rc::new(Cell::new(false));
    // Where the pointer was among the tabs when the tab was picked up.
    let start = Rc::new(Cell::new(0.0_f32));
    let (down, pressed, granted) = (dragged.clone(), dragged.clone(), dragged);
    let (grant_start, move_start) = (start.clone(), start);
    let (grant_widths, move_widths) = (widths.clone(), widths);
    let (release, act) = (bar.clone(), bar.clone());
    // Assistive technology moves a tab a place at a time.
    let actions = vec![
        AccessibilityActionDef {
            name: "moveLeft".into(),
            label: Some("Move list left".into()),
        },
        AccessibilityActionDef {
            name: "moveRight".into(),
            label: Some("Move list right".into()),
        },
    ];
    label
        .accessibility_actions(actions)
        .on_accessibility_action(move |name: &str| {
            let by = match name {
                "moveLeft" => -1,
                "moveRight" => 1,
                _ => return,
            };
            act.save(|settings| settings.moved(at, by));
        })
        .cursor(Cursor::Grab)
        .on_press_down(move || down.set(false))
        .on_press(move || {
            if !pressed.replace(false) {
                on_press();
            }
        })
        .claim_touches(TouchClaim::on_pan(PanActivation {
            active_offset_x: Some(px(SLOP)),
            active_offset_y: Some(px(SLOP)),
            ..PanActivation::default()
        }))
        .keep_touches(true)
        .on_responder_grant(move |event: &ResponderEvent| {
            granted.set(true);
            let left: f32 = grant_widths.borrow().iter().take(at).sum();
            grant_start.set(left + event.location.x.0);
            drag.set(Some(Drag { from: at, gap: at }));
        })
        .on_responder_move(move |event: &ResponderEvent| {
            let x = move_start.get() + event.gesture.delta.x.0;
            let gap = gap_at(&move_widths.borrow(), x);
            let next = Some(Drag { from: at, gap });
            if drag.get_untracked() != next {
                drag.set(next);
            }
        })
        .on_responder_release(move |_: &ResponderEvent| {
            let Some(held) = drag.get_untracked() else {
                return;
            };
            batch(|| {
                drag.set(None);
                if let Some(to) = held.to() {
                    release.close_menu();
                    release.save(|settings| settings.moved(at, to as isize - at as isize));
                }
            });
        })
        .on_responder_terminate(move |_: &ResponderEvent| drag.set(None))
}

/// The bar over the leading edge of the tab after gap `gap` (or the
/// trailing edge of the last tab: `after`) while the tab dragged would go
/// there and move. It takes no room.
fn gap_bar(drag: Signal<Option<Drag>>, gap: usize, after: bool) -> View {
    let shown =
        move || drag.with(|drag| drag.is_some_and(|drag| drag.gap == gap && drag.to().is_some()));
    let bar = view()
        .absolute()
        .top(px(0.0))
        .bottom(px(0.0))
        .w(px(GAP_BAR))
        .bg(theme::ACCENT)
        .pointer_events(PointerEvents::None)
        .test_id(&format!("watchlist-drop-{gap}"))
        .style_when(move || !shown(), |s| s.hidden());
    if after {
        bar.right(px(0.0))
    } else {
        bar.left(px(0.0))
    }
}

/// The field a list's name is edited in: Enter or leaving it saves the
/// name typed (a blank one keeps the old), and ends the editing.
fn rename_field(bar: &Bar, at: usize, name: String) -> AnyElement {
    let draft = signal(name);
    let commit = {
        let bar = bar.clone();
        Rc::new(move || {
            if bar.renaming.get_untracked() != Some(at) {
                return;
            }
            let typed = draft.get_untracked();
            batch(|| {
                bar.renaming.set(None);
                bar.save(|settings| Some(settings.renamed(at, &typed)));
            });
        })
    };
    let (submit, blur) = (commit.clone(), commit);
    ui::text_field(draft)
        .h_full()
        .w(px(RENAME_WIDTH))
        .flex_none()
        .auto_focus(true)
        .max_length(NAME_LENGTH as u32)
        .placeholder("List name")
        .label("List name")
        .test_id(&format!("watchlist-rename-{at}"))
        .on_submit(move |_: &str| submit())
        .on_blur(move || blur())
        .into_element()
}

/// A tab's ▾: opens the menu of its list, hanging from it.
fn menu_cell(bar: &Bar, at: usize, name: String, count: usize) -> View {
    let anchor = element_ref();
    let menu = bar.menu;
    let open = move || menu.get() == Some(at);
    let toggle = {
        let bar = bar.clone();
        move || {
            if menu.get_untracked() == Some(at) {
                bar.close_menu();
            } else {
                batch(|| {
                    bar.confirming.set(false);
                    menu.set(Some(at));
                });
            }
        }
    };
    let overlay = {
        let bar = bar.clone();
        dynamic(move || open().then(|| list_menu(&bar, &anchor, at, count)))
    };
    view()
        .h_full()
        .w(px(MENU_CELL))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .element_ref(&anchor)
        .text_size(px(10.0))
        .text_color(theme::TEXT_FAINT)
        .style_when(open, |s| s.text_color(theme::ACCENT))
        .role(AccessibilityRole::Button)
        .label(format!("Options for list {name}"))
        .test_id(&format!("watchlist-tab-menu-{at}"))
        .hover(|s| s.text_color(theme::TEXT))
        .active(|s| s.bg(theme::BORDER))
        .on_press(toggle)
        .child(text("▾"))
        .child(overlay)
}

/// The menu of the list at `at`, below its ▾.
fn list_menu(bar: &Bar, anchor: &ElementRef, at: usize, count: usize) -> Overlay {
    let rename = {
        let bar = bar.clone();
        menu_row("Rename", false, true, move || {
            bar.close_menu();
            bar.renaming.set(Some(at));
        })
    };
    let confirming = bar.confirming;
    let remove = {
        let bar = bar.clone();
        menu_row("Remove", true, count > 1, move || {
            if !confirming.get_untracked() {
                confirming.set(true);
                return;
            }
            bar.close_menu();
            bar.refusal.set(None);
            bar.save(|settings| settings.removed(at));
        })
        .style_when(move || confirming.get(), |s| s.text_color(theme::DANGER))
        .child(text(move || if confirming.get() { "?" } else { "" }).text_color(theme::DANGER))
    };
    let close = {
        let bar = bar.clone();
        move || bar.close_menu()
    };
    let cancel = close.clone();
    overlay()
        .anchor(anchor)
        .placement(Placement::Bottom)
        .align(Align::Start)
        .offset(px(1.0))
        .on_press_outside(move |_: &Point<Points>| close())
        .on_cancel(cancel)
        .child(
            view()
                .test_id("watchlist-list-menu")
                .min_w(px(MENU_WIDTH))
                .flex_col()
                .py(px(4.0))
                .bg(theme::PANEL)
                .border_1()
                .border_color(theme::BORDER)
                .text_size(px(11.0))
                .text_color(theme::TEXT)
                .role(AccessibilityRole::Menu)
                .label("List options")
                .child(rename)
                .child(remove),
        )
}

/// One row of a list's menu: `label`, pressed to `act` (at half opacity
/// and taking no press while not `enabled`). A `destructive` one is muted,
/// and red under the pointer.
fn menu_row(
    label: &'static str,
    destructive: bool,
    enabled: bool,
    act: impl Fn() + 'static,
) -> View {
    view()
        .h(px(MENU_ROW))
        .w_full()
        .flex_row()
        .items_center()
        .px(px(10.0))
        .text_color(if destructive {
            theme::TEXT_MUTED
        } else {
            theme::TEXT
        })
        .role(AccessibilityRole::Menuitem)
        .label(label)
        .test_id(&format!("watchlist-list-{}", label.to_ascii_lowercase()))
        .disabled(move || !enabled)
        .disabled_style(|s| s.opacity(0.5))
        .hover(move |s| {
            let s = s.bg(theme::SURFACE);
            if destructive {
                s.text_color(theme::DANGER)
            } else {
                s
            }
        })
        .active(|s| s.bg(theme::BORDER))
        .on_press(move || {
            if enabled {
                act();
            }
        })
        .child(text(label).number_of_lines(1))
}

/// `+ New List`: a new empty list, shown, its name open for editing.
fn new_list(bar: &Bar) -> View {
    let bar = bar.clone();
    view()
        .h_full()
        .flex_none()
        .flex()
        .items_center()
        .px(px(TAB_PADDING))
        .border_r_1()
        .border_color(theme::BORDER)
        .text_size(px(11.0))
        .text_color(theme::TEXT_MUTED)
        .role(AccessibilityRole::Button)
        .label("New list")
        .test_id("watchlist-new-list")
        .hover(|s| s.bg(theme::PANEL).text_color(theme::TEXT))
        .active(|s| s.bg(theme::BORDER))
        .on_press(move || {
            let next = bar.tab.settings.get_untracked().with_new_list();
            let at = next.active();
            batch(|| {
                bar.close_menu();
                bar.refusal.set(None);
                bar.tab.save(&next);
                bar.renaming.set(Some(at));
            });
        })
        .child(text("+ New List").number_of_lines(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dragged_tab_goes_to_the_gap_under_the_pointer() {
        let widths = [60.0, 80.0, 40.0];
        assert_eq!(gap_at(&widths, 10.0), 0);
        assert_eq!(gap_at(&widths, 31.0), 1);
        assert_eq!(gap_at(&widths, 101.0), 2);
        assert_eq!(gap_at(&widths, 500.0), 3);
        // Either gap beside it leaves it where it is.
        assert_eq!(Drag { from: 1, gap: 1 }.to(), None);
        assert_eq!(Drag { from: 1, gap: 2 }.to(), None);
        assert_eq!(Drag { from: 1, gap: 0 }.to(), Some(0));
        assert_eq!(Drag { from: 0, gap: 3 }.to(), Some(2));
    }
}
