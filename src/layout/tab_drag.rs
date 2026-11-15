//! Dragging a tab to another place among its page's tabs: along its own
//! pane's, or into another pane, as a browser's tabs go and the old app's
//! did (`WidgetTabDragContext.tsx`, `TabbedWidget.tsx`).
//!
//! A tab dragged past a slop is picked up ([`Held`]): it dims where it was,
//! its title follows the pointer, and the pane under the pointer shows where
//! it would go, outlined, with a bar at the gap among its tabs: before the
//! first tab whose middle is past the pointer while it is over them, after
//! the last one anywhere else on the pane. Lifting the pointer there moves
//! the tab ([`super::move_tab`]); lifting it off every pane, Escape, or a
//! change that takes the tab or its page away puts it back. Nothing is saved
//! before the drop.
//!
//! NSH has no drop targets: the tab dragged hears the whole gesture (the
//! touch responder, which carries mouse drags too), so the pane and the gap
//! under the pointer are measured, from the layout as it is then (scrolled,
//! resized), by one task that goes on until it has caught up with the
//! pointer. Held at an edge of what scrolls (a pane's tabs that reach past
//! their room, the grid when it is higher than its room), the tab scrolls it
//! that way, so every place on the page can be reached in one drag.

use std::{cell::Cell, rc::Rc};

use nsh_core::prelude::*;

use super::{StoredWorkspace, active_grid, grid::Tab, move_tab, persist};
use crate::theme;

/// How far a tab is dragged before it is picked up rather than pressed.
const SLOP: f32 = 6.0;
/// How far sideways a touch goes, over tabs it pans, before it is that pan
/// rather than a tab pulled out of them.
const PAN: f32 = 10.0;
/// How near an edge of what scrolls (a pane's tabs sideways, the grid up
/// and down) the pointer scrolls it.
const EDGE: f32 = 28.0;
/// How far it scrolls a pane's tabs a frame.
const TABS_STEP: f32 = 8.0;
/// How far it scrolls the grid a frame.
const GRID_STEP: f32 = 10.0;
/// Where the pointer is in the title that follows it, from its top left
/// corner (the old app's drag image).
const GRIP: f32 = 16.0;
/// The bar that marks a gap.
const BAR: f32 = 3.0;

/// The tab in hand.
#[derive(Clone, Debug, PartialEq)]
struct Held {
    page: usize,
    pane: usize,
    tab: usize,
    title: String,
    /// Where it would go if dropped now: a pane and a gap among its tabs
    /// ([`super::Grid::move_tab`]).
    target: Option<(usize, usize)>,
}

impl Held {
    /// Whether the tab is still where it was picked up, on the page shown.
    fn valid(&self, saved: &StoredWorkspace) -> bool {
        saved.active_page_id == self.page
            && active_grid(saved)
                .layouts
                .iter()
                .flatten()
                .any(|pane| pane.id == self.pane && pane.tabs.iter().any(|tab| tab.id == self.tab))
    }
}

/// What a pane on screen gives the drag to find a tab's place in it.
#[derive(Clone)]
struct DropZone {
    /// Its whole box.
    frame: ElementRef,
    /// Its tabs' scroll view.
    strip: ElementRef,
    /// Its tabs, in order.
    tabs: Stored<Vec<ElementRef>>,
    /// Moves its tabs sideways by so many points; whether they moved.
    scroll: Rc<dyn Fn(f32) -> bool>,
}

/// The grid's tab drags: the tab in hand and the panes it can go to.
#[derive(Clone, Copy)]
pub(super) struct TabDrags {
    workspace: Signal<StoredWorkspace>,
    held: Signal<Option<Held>>,
    /// Where the pointer holding it is in the window.
    pointer: Signal<Option<Point<Points>>>,
    /// The pointer lifted: the tab is dropped once its place there is known.
    released: Stored<bool>,
    /// Whether the task that finds its place runs.
    looking: Stored<bool>,
    /// The panes on screen, by page and pane id.
    zones: Stored<Vec<((usize, usize), DropZone)>>,
    /// The grid's scroll view: a pane scrolled out of it takes no tab.
    viewport: ElementRef,
    /// What it scrolls: the box the panes are placed in.
    content: ElementRef,
    /// The grid's owner, which that task runs under: a listener's own is
    /// its tab's, made again with the pane's tabs.
    owner: Owner,
}

impl TabDrags {
    /// The drags of the grid: `viewport` is its scroll view and `content`
    /// the box in it that the panes are placed in. Owned by the current
    /// owner.
    pub(super) fn new(
        workspace: Signal<StoredWorkspace>,
        viewport: ElementRef,
        content: ElementRef,
    ) -> Self {
        let drags = Self {
            workspace,
            held: signal(None),
            pointer: signal(None),
            released: stored(false),
            looking: stored(false),
            zones: stored(Vec::new()),
            viewport,
            content,
            owner: Owner::current(),
        };
        let held = drags.held;
        // A change that takes the tab or its page away puts it back: a drop
        // would move whatever tab has its ids on the page shown then.
        effect(move || {
            let stale = workspace.with(|saved| {
                held.with_untracked(|held| held.as_ref().is_some_and(|held| !held.valid(saved)))
            });
            if stale {
                drags.cancel();
            }
        });
        // Escape puts it back too, and goes on to what else it closes.
        let holding = computed(move || held.with(Option::is_some));
        effect(move || {
            if holding.get() {
                let passed = KeyOptions {
                    pass_through: true,
                    ..KeyOptions::default()
                };
                on_key_with("escape", passed, move |_| drags.cancel());
            }
        });
        drags
    }

    /// Pane `pane` of `page`'s part in the drags, for as long as the current
    /// owner (its view) lives: `frame` is its whole box and `strip` its
    /// tabs' scroll view, which `scroll` moves sideways by so many points,
    /// saying whether it moved.
    pub(super) fn pane(
        self,
        page: usize,
        pane: usize,
        frame: ElementRef,
        strip: ElementRef,
        scroll: impl Fn(f32) -> bool + 'static,
    ) -> PaneDrag {
        let tabs = stored(Vec::new());
        let zone = DropZone {
            frame,
            strip,
            tabs,
            scroll: Rc::new(scroll),
        };
        self.zones.with(|zones| zones.push(((page, pane), zone)));
        // The grid may go first.
        cleanup(move || {
            self.zones
                .try_with(|zones| zones.retain(|(key, _)| *key != (page, pane)));
        });
        let held = self.held;
        PaneDrag {
            drags: self,
            page,
            pane,
            gap: computed(move || {
                held.with(|held| {
                    let held = held.as_ref().filter(|held| held.page == page)?;
                    let (to, gap) = held.target?;
                    (to == pane).then_some(gap)
                })
            }),
            held: computed(move || {
                held.with(|held| {
                    let held = held.as_ref()?;
                    (held.page == page && held.pane == pane).then_some(held.tab)
                })
            }),
            tabs,
        }
    }

    /// The title of the tab in hand, beside the pointer, as the old app's
    /// drag image was. It takes no pointer: the tab under it is the one
    /// that hears the drag.
    pub(super) fn ghost(self) -> If {
        let Self { held, pointer, .. } = self;
        let title = move || {
            held.with_untracked(|held| held.as_ref().map(|held| held.title.clone()))
                .unwrap_or_default()
        };
        let at = move |axis: fn(Point<Points>) -> Points| {
            px(pointer.get().map_or(0.0, |at| axis(at).0 - GRIP))
        };
        when(
            move || held.with(Option::is_some),
            move || {
                overlay()
                    .left(move || at(|at| at.x))
                    .top(move || at(|at| at.y))
                    .pointer_events(PointerEvents::None)
                    .child(
                        view()
                            .h(px(32.0))
                            .flex()
                            .items_center()
                            .px(px(12.0))
                            .text_size(px(11.0))
                            .text_color(theme::TEXT)
                            .bg(theme::BACKGROUND)
                            .border_1()
                            .border_color(theme::ACCENT)
                            .opacity(0.9_f32)
                            .pointer_events(PointerEvents::None)
                            .test_id("tab-drag-ghost")
                            .child(text(title())),
                    )
            },
        )
    }

    /// A drag picked `held` up, with the pointer at `at`.
    fn pick_up(self, held: Held, at: Point<Points>) {
        batch(|| {
            self.released.replace(false);
            self.held.set(Some(held));
            self.pointer.set(Some(at));
        });
        self.look();
    }

    /// The pointer holding the tab moved to `at`.
    fn follow(self, at: Point<Points>) {
        // Put back since (Escape): the rest of the gesture is nothing's.
        if self.held.with_untracked(Option::is_some) {
            self.pointer.set(Some(at));
            self.look();
        }
    }

    /// The pointer lifted at `at`: the tab goes where it would there.
    fn release(self, at: Point<Points>) {
        if self.held.with_untracked(Option::is_some) {
            self.pointer.set(Some(at));
            self.released.replace(true);
            self.look();
        }
    }

    /// Puts the tab in hand back where it was.
    fn cancel(self) {
        batch(|| {
            self.held.set(None);
            self.pointer.set(None);
            self.released.replace(false);
        });
    }

    /// Finds the tab's place under the pointer, again until the pointer has
    /// not moved on meanwhile and nothing scrolls under it, and drops the
    /// tab there once the pointer has lifted. One task at a time.
    fn look(self) {
        if self.looking.replace(true) {
            return;
        }
        self.owner.run(|| {
            spawn(async move {
                loop {
                    let page = self
                        .held
                        .with_untracked(|held| held.as_ref().map(|held| held.page));
                    let (Some(page), Some(at)) = (page, self.pointer.get_untracked()) else {
                        break;
                    };
                    let released = self.released.get();
                    let (target, scrolled) = self.target(page, at, !released).await;
                    // Put back while it was measured.
                    let Some(mut held) = self.held.get_untracked() else {
                        break;
                    };
                    held.target = target;
                    if released {
                        self.drop_tab(&held);
                        break;
                    }
                    // Signal::set compares values: moves within a gap redraw
                    // nothing.
                    self.held.set(Some(held));
                    if scrolled {
                        next_frame().await;
                    } else if self.pointer.get_untracked() == Some(at) && !self.released.get() {
                        break;
                    }
                }
                self.looking.replace(false);
            });
        });
    }

    /// Moves the tab that was in hand to where it would go, and saves.
    fn drop_tab(self, held: &Held) {
        batch(|| {
            self.cancel();
            let Some((to, gap)) = held.target else {
                return;
            };
            let mut saved = self.workspace.get_untracked();
            if held.valid(&saved) && move_tab(&mut saved, held.pane, held.tab, to, gap) {
                persist(&saved);
                self.workspace.set(saved);
            }
        });
    }

    /// Where a tab dropped at `at` (in the window) would go on `page`: the
    /// pane there and a gap among its tabs, as the old app chose it: before
    /// the first tab whose middle is past the pointer while it is over the
    /// tabs, after the last one anywhere else on the pane. With `scroll`,
    /// what the pointer is at an edge of scrolls that way (the grid, the
    /// pane's tabs); whether anything did, and so is elsewhere next frame.
    async fn target(
        self,
        page: usize,
        at: Point<Points>,
        scroll: bool,
    ) -> (Option<(usize, usize)>, bool) {
        let inside = |bounds: Option<Bounds<Points>>| bounds.is_some_and(|b| b.contains(&at));
        let Some(viewport) = page_bounds(self.viewport).await.filter(|b| b.contains(&at)) else {
            return (None, false);
        };
        // The grid is scrolled by how far its content starts above its room.
        let content = page_bounds(self.content).await;
        let scrolled_grid = scroll
            && content.is_some_and(|content| {
                let from = viewport.top().0 - content.top().0;
                let by = if at.y.0 - viewport.top().0 < EDGE {
                    -GRID_STEP
                } else if viewport.bottom().0 - at.y.0 < EDGE {
                    GRID_STEP
                } else {
                    0.0
                };
                let most = (content.size.height.0 - viewport.size.height.0).max(0.0);
                let to = (from + by).clamp(0.0, most);
                if to == from {
                    return false;
                }
                self.viewport.scroll_to(point(px(0.0), px(to)), false);
                true
            });
        let zones = self.zones.with(|zones| {
            let on_page = zones.iter().filter(|((on, _), _)| *on == page);
            on_page
                .map(|((_, pane), zone)| (*pane, zone.clone()))
                .collect::<Vec<_>>()
        });
        for (pane, zone) in zones {
            if !inside(page_bounds(zone.frame).await) {
                continue;
            }
            let strip = page_bounds(zone.strip).await;
            // Nothing is awaited from here to the scroll: a pane whose tabs
            // are still kept is still there to scroll.
            let Some(tabs) = zone.tabs.try_with(|tabs| tabs.clone()) else {
                continue;
            };
            let Some(strip) = strip.filter(|strip| strip.contains(&at)) else {
                return (Some((pane, tabs.len())), scrolled_grid);
            };
            let scrolled_tabs = scroll
                && if at.x.0 - strip.left().0 < EDGE {
                    (zone.scroll)(-TABS_STEP)
                } else if strip.right().0 - at.x.0 < EDGE {
                    (zone.scroll)(TABS_STEP)
                } else {
                    false
                };
            let mut gap = tabs.len();
            for (index, tab) in tabs.iter().enumerate() {
                if page_bounds(*tab)
                    .await
                    .is_some_and(|tab| at.x < tab.center().x)
                {
                    gap = index;
                    break;
                }
            }
            return (Some((pane, gap)), scrolled_grid || scrolled_tabs);
        }
        (None, scrolled_grid)
    }
}

/// Where `element` is in the window, in a pointer's coordinates.
async fn page_bounds(element: ElementRef) -> Option<Bounds<Points>> {
    let at = element.measure().await?;
    Some(Bounds {
        origin: at.page,
        size: at.frame.size,
    })
}

/// When a drag picks a tab up: once it has gone the slop, whichever way. A
/// touch drags tabs that reach past their room (`overflows`) sideways to pan
/// them, so there it picks one up by pulling it out of them. (A mouse pans
/// nothing: the wheel moves them.)
fn activation(overflows: bool) -> PanActivation {
    let slop = Some(px(SLOP));
    if overflows && !is_desktop() {
        PanActivation::vertical(px(SLOP), px(PAN))
    } else {
        PanActivation {
            active_offset_x: slop,
            active_offset_y: slop,
            ..PanActivation::default()
        }
    }
}

/// A pane's part in the tab drags ([`TabDrags::pane`]).
#[derive(Clone, Copy)]
pub(super) struct PaneDrag {
    drags: TabDrags,
    page: usize,
    pane: usize,
    /// The gap among its tabs where the tab in hand would go.
    gap: Memo<Option<usize>>,
    /// Its tab in hand.
    held: Memo<Option<usize>>,
    /// Its tabs' elements, in order.
    tabs: Stored<Vec<ElementRef>>,
}

impl PaneDrag {
    /// Forgets the pane's tabs: they are being made again, each of them a
    /// [`Self::tab`], in order.
    pub(super) fn remake(self) {
        self.tabs.with(Vec::clear);
    }

    /// Whether the tab in hand would go to this pane (tracked).
    pub(super) fn targeted(self) -> bool {
        self.gap.get().is_some()
    }

    /// Makes `view` the pane's next tab, `tab`: one a drag picks up
    /// (`overflows`: the pane's tabs reach past their room), dimmed while it
    /// is in hand, with the bar of the gap before it (and of the one after
    /// it on the `last` tab) while the tab in hand would go there. A press
    /// that ends a drag is none of the tab: `on_press` hears the others.
    pub(super) fn tab(
        self,
        view: View,
        tab: &Tab,
        last: bool,
        overflows: bool,
        on_press: impl Fn() + 'static,
    ) -> View {
        let Self {
            drags,
            page,
            pane,
            gap,
            held,
            tabs,
        } = self;
        let at = element_ref();
        let index = tabs.with(|tabs| {
            tabs.push(at);
            tabs.len() - 1
        });
        let (tab, title) = (tab.id, tab.title.clone());
        let dragged = Rc::new(Cell::new(false));
        let (down, pressed, granted) = (dragged.clone(), dragged.clone(), dragged);
        let view = view
            .element_ref(&at)
            .relative()
            .cursor(Cursor::Grab)
            .opacity(move || {
                if held.get() == Some(tab) {
                    0.4_f32
                } else {
                    1.0
                }
            })
            .on_press_down(move || down.set(false))
            .on_press(move || {
                if !pressed.replace(false) {
                    on_press();
                }
            })
            .claim_touches(TouchClaim::on_pan(activation(overflows)))
            .keep_touches(true)
            .on_responder_grant(move |event: &ResponderEvent| {
                granted.set(true);
                let held = Held {
                    page,
                    pane,
                    tab,
                    title: title.clone(),
                    target: None,
                };
                drags.pick_up(held, event.page);
            })
            .on_responder_move(move |event: &ResponderEvent| drags.follow(event.page))
            .on_responder_release(move |event: &ResponderEvent| drags.release(event.page))
            .on_responder_terminate(move |_: &ResponderEvent| drags.cancel())
            .child(bar(pane, index, false, move || gap.get() == Some(index)));
        if last {
            view.child(bar(pane, index + 1, true, move || {
                gap.get() == Some(index + 1)
            }))
        } else {
            view
        }
    }
}

/// The bar that marks gap `gap` of pane `pane` while `shown`: over the
/// leading edge of the tab after the gap, or the trailing edge of the last
/// tab (`after`). It takes no room: the tabs stay where they are.
fn bar(pane: usize, gap: usize, after: bool, shown: impl Fn() -> bool + 'static) -> If {
    when(shown, move || {
        let bar = view()
            .absolute()
            .top(px(0.0))
            .bottom(px(0.0))
            .w(px(BAR))
            .bg(theme::ACCENT)
            .pointer_events(PointerEvents::None)
            .test_id(&format!("pane-drop-{pane}-{gap}"));
        if after {
            bar.right(px(0.0))
        } else {
            bar.left(px(0.0))
        }
    })
}

/// "Drop tab here", over the content of a pane without tabs, while `shown`
/// (the tab in hand would go to it), as the old app's empty pane said.
pub(super) fn drop_hint(shown: impl Fn() -> bool + 'static) -> If {
    when(shown, || {
        view()
            .absolute()
            .top(px(8.0))
            .right(px(8.0))
            .bottom(px(8.0))
            .left(px(8.0))
            .flex()
            .items_center()
            .justify_center()
            .border_1()
            .border_dashed()
            .border_color(theme::ACCENT)
            .bg(theme::SURFACE)
            .text_size(px(12.0))
            .text_color(theme::ACCENT)
            .pointer_events(PointerEvents::None)
            .child(text("Drop tab here"))
    })
}
