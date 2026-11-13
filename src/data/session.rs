//! Widget sessions: a widget's data lives as long as the widget is in the
//! workspace, not as long as it is on screen.
//!
//! A widget's view is made and disposed often: another page is shown, the
//! app starts with the widget on a page nobody opened yet. What it streams
//! and keeps (subscriptions, watches, buffers, sort, selection) must not go
//! with the view, or a hidden widget would stop streaming and start over
//! when shown. So that state lives in the widget's *session*:
//!
//! - One session per widget, keyed by its [`WidgetId`], made the first time
//!   a view of the widget attaches ([`Sessions::attach`]).
//! - A session has an NSH owner of its own (a root under the data services,
//!   so it sees their context), not a view's: what its state makes (signals,
//!   effects, a `Subscription::release_on_cleanup`, a task from `spawn`)
//!   lives as long as the session.
//! - Views attach and detach; the session knows whether any attached view
//!   is visible ([`SessionScope::is_visible`]). Rendering-only work (frame
//!   batching, display clocks, copying data into view signals) goes in
//!   [`SessionScope::while_visible`], which runs it only while it shows.
//! - The registry follows the workspace ([`Sessions::follow`]): a session
//!   is released when its widget leaves the workspace (tab closed, pane
//!   removed, page cleared or deleted, a layout imported), and disposed once
//!   its last view has detached too. Widgets never release anything.
//!
//! A widget gets its state with [`WidgetView::state`]: get-or-create one
//! value of a type per session, made under the session's owner on first use
//! and the same `Rc` for every later view.

use std::{
    any::{Any, TypeId, type_name},
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    fmt,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

use nsh_core::{element::create_root, prelude::*};

/// One widget instance, for this run of the app: what its session is keyed
/// by.
///
/// Not saved. Every tab that enters the workspace (loaded, imported or
/// created) gets a fresh id that no other widget of this run had, so a
/// widget made after an import or a page clear can never inherit an old
/// widget's session, even when the layout reuses page and tab ids. A
/// widget's id must change whenever it stops being the same widget (when
/// tabs get a widget type, changing it makes a new id).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WidgetId(u64);

impl WidgetId {
    /// An id no widget of this run had.
    pub fn fresh() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

impl fmt::Display for WidgetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "w{}", self.0)
    }
}

/// Every widget's session. Cloning is cheap and shares the registry.
#[derive(Clone)]
pub struct Sessions(Rc<Registry>);

struct Registry {
    /// What sessions are made under: their roots see its context.
    owner: Owner,
    sessions: RefCell<HashMap<WidgetId, Rc<Core>>>,
    next_view: Cell<u64>,
}

struct Core {
    widget: WidgetId,
    /// Everything the session made; disposed when the session is.
    owner: Owner,
    /// Whether any attached view is visible (owned by `owner`).
    visible: Signal<bool>,
    /// The attached views and whether each is visible.
    views: RefCell<Vec<(u64, bool)>>,
    /// The widget's states, one per type.
    states: RefCell<HashMap<TypeId, Rc<dyn Any>>>,
    /// The states being made right now (a state asking for itself).
    making: RefCell<Vec<TypeId>>,
    /// Its widget left the workspace; disposed once no view is attached.
    released: Cell<bool>,
    disposed: Cell<bool>,
}

impl Default for Sessions {
    fn default() -> Self {
        Self::new()
    }
}

impl Sessions {
    /// A registry whose sessions are made under the current owner (the
    /// data services'), and see its context.
    pub fn new() -> Self {
        Self(Rc::new(Registry {
            owner: Owner::current(),
            sessions: RefCell::new(HashMap::new()),
            next_view: Cell::new(0),
        }))
    }

    /// Attaches a view of `widget` to its session, made now if this is the
    /// widget's first view. `visible` (a value, signal or closure) is
    /// whether this view shows; it is followed under the current owner (the
    /// view's), so call this while making the view. The view stays attached
    /// until the returned handle drops ([`WidgetView::release_on_cleanup`]
    /// ties it to the view).
    pub fn attach(&self, widget: WidgetId, visible: impl IntoReactive<bool>) -> WidgetView {
        let core = self.session(widget);
        let id = self.0.next_view.get() + 1;
        self.0.next_view.set(id);
        let visible = visible.into_reactive();
        let shown = untracked(|| visible.get());
        core.views.borrow_mut().push((id, shown));
        core.refresh();
        let weak = Rc::downgrade(&core);
        effect(move || {
            let shown = visible.get();
            if let Some(core) = weak.upgrade() {
                untracked(|| core.show(id, shown));
            }
        });
        WidgetView { core, id }
    }

    /// Releases the session of every widget not in `live`.
    pub fn retain(&self, live: &HashSet<WidgetId>) {
        let gone: Vec<Rc<Core>> = self
            .0
            .sessions
            .borrow_mut()
            .extract_if(|widget, _| !live.contains(widget))
            .map(|(_, core)| core)
            .collect();
        // Outside the borrow: disposing runs the sessions' cleanups.
        for core in gone {
            core.release();
        }
    }

    /// Keeps the sessions in step with the workspace: `live` (the widgets
    /// in it) runs now and whenever what it reads changes, and every other
    /// session is released ([`Self::retain`]). Owned by the current owner.
    ///
    /// It is an effect, so it runs after the views of a change were made
    /// again: a closed tab's view has detached by the time its session is
    /// released.
    pub fn follow(&self, live: impl Fn() -> HashSet<WidgetId> + 'static) {
        let sessions = self.clone();
        effect(move || {
            let live = live();
            untracked(|| sessions.retain(&live));
        });
    }

    /// How many sessions there are.
    pub fn len(&self) -> usize {
        self.0.sessions.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether `widget` has a session.
    pub fn contains(&self, widget: WidgetId) -> bool {
        self.0.sessions.borrow().contains_key(&widget)
    }

    /// Whether `widget`'s session has a visible view (tracked); `None`
    /// without a session.
    pub fn is_visible(&self, widget: WidgetId) -> Option<bool> {
        let core = self.0.sessions.borrow().get(&widget).cloned()?;
        Some(core.visible.try_get().unwrap_or(false))
    }

    /// Disposes every session (the app is going away).
    pub(crate) fn shutdown(&self) {
        let all: Vec<Rc<Core>> = self
            .0
            .sessions
            .borrow_mut()
            .drain()
            .map(|(_, core)| core)
            .collect();
        for core in all {
            core.released.set(true);
            core.dispose();
        }
    }

    /// `widget`'s session, made if it has none.
    fn session(&self, widget: WidgetId) -> Rc<Core> {
        if let Some(core) = self.0.sessions.borrow().get(&widget) {
            return core.clone();
        }
        let (owner, visible) = self.0.owner.run(|| create_root(|| signal(false)));
        let core = Rc::new(Core {
            widget,
            owner,
            visible,
            views: RefCell::new(Vec::new()),
            states: RefCell::new(HashMap::new()),
            making: RefCell::new(Vec::new()),
            released: Cell::new(false),
            disposed: Cell::new(false),
        });
        self.0.sessions.borrow_mut().insert(widget, core.clone());
        log::debug!("Lattice: session of widget {widget} made");
        core
    }
}

impl Core {
    fn scope(&self) -> SessionScope {
        SessionScope {
            widget: self.widget,
            owner: self.owner,
            visible: self.visible.read_only(),
        }
    }

    /// Sets the session's visibility from its views'.
    fn refresh(&self) {
        let visible = self.views.borrow().iter().any(|(_, shown)| *shown);
        self.visible.try_set(visible);
    }

    fn show(&self, view: u64, shown: bool) {
        let changed = match self
            .views
            .borrow_mut()
            .iter_mut()
            .find(|(id, _)| *id == view)
        {
            Some((_, was)) if *was != shown => {
                *was = shown;
                true
            }
            _ => false,
        };
        if changed {
            self.refresh();
        }
    }

    fn detach(&self, view: u64) {
        self.views.borrow_mut().retain(|(id, _)| *id != view);
        self.refresh();
        if self.released.get() && self.views.borrow().is_empty() {
            self.dispose();
        }
    }

    fn release(&self) {
        self.released.set(true);
        log::debug!("Lattice: session of widget {} released", self.widget);
        // A view still attached keeps using it until it detaches.
        if self.views.borrow().is_empty() {
            self.dispose();
        }
    }

    fn dispose(&self) {
        if self.disposed.replace(true) {
            return;
        }
        // Its effects, subscriptions and tasks end first, then the states.
        self.owner.dispose();
        let states = std::mem::take(&mut *self.states.borrow_mut());
        drop(states);
    }
}

/// One view's attachment to its widget's session: how the view gets the
/// widget's state. The session outlives every view, but is never disposed
/// while one is attached. Dropping it detaches the view.
#[must_use = "dropping a widget view detaches it at once"]
pub struct WidgetView {
    core: Rc<Core>,
    id: u64,
}

impl WidgetView {
    pub fn widget(&self) -> WidgetId {
        self.core.widget
    }

    /// The widget's state of type `S`: made by `init` under the session's
    /// owner the first time any view asks, and the same value for every
    /// view after, as long as the widget is in the workspace. `init` may
    /// keep `scope` for later (it is `Copy`).
    ///
    /// # Panics
    ///
    /// When `init` asks for the state it is making.
    pub fn state<S: 'static>(&self, init: impl FnOnce(SessionScope) -> S) -> Rc<S> {
        let core = &self.core;
        let ty = TypeId::of::<S>();
        let made = core.states.borrow().get(&ty).cloned();
        if let Some(state) = made.and_then(|state| state.downcast::<S>().ok()) {
            return state;
        }
        assert!(
            !core.making.borrow().contains(&ty),
            "the session state `{}` of widget {} asked for itself while being made",
            type_name::<S>(),
            core.widget
        );
        /// Takes the type off `making`, on a panic in `init` too.
        struct Making<'a>(&'a Core, TypeId);
        impl Drop for Making<'_> {
            fn drop(&mut self) {
                self.0.making.borrow_mut().retain(|ty| *ty != self.1);
            }
        }
        core.making.borrow_mut().push(ty);
        let _making = Making(core, ty);
        let scope = core.scope();
        let state = Rc::new(core.owner.run(|| init(scope)));
        core.states
            .borrow_mut()
            .insert(ty, state.clone() as Rc<dyn Any>);
        state
    }

    /// The session's handle (see [`SessionScope`]).
    pub fn scope(&self) -> SessionScope {
        self.core.scope()
    }

    /// Keeps the view attached until the current owner (the view) is
    /// cleaned up, instead of until this handle drops.
    pub fn release_on_cleanup(self) {
        cleanup(move || drop(self));
    }
}

impl Drop for WidgetView {
    fn drop(&mut self) {
        self.core.detach(self.id);
    }
}

/// A session's handle for its state: which widget, whether it shows, and
/// a place to make what must live as long as the session. `Copy`, so a
/// state can keep it. Once the session is gone, it reads as hidden and
/// what is made under it is disposed at once.
#[derive(Clone, Copy)]
pub struct SessionScope {
    widget: WidgetId,
    owner: Owner,
    visible: ReadSignal<bool>,
}

impl SessionScope {
    pub fn widget(&self) -> WidgetId {
        self.widget
    }

    /// Whether any attached view of the widget is visible (tracked).
    pub fn is_visible(&self) -> bool {
        self.visible.try_get().unwrap_or(false)
    }

    /// Runs `work` each time the widget becomes visible, under an owner
    /// disposed when it hides or the session goes: the place for rendering
    /// only work (per-frame batching, display clocks, copying data into
    /// view signals), so a hidden widget keeps its data but does none of it.
    /// `work` runs untracked; what it starts is its own business to stop
    /// with `cleanup`.
    pub fn while_visible(&self, work: impl Fn() + 'static) {
        let visible = self.visible;
        self.owner.run(|| {
            effect(move || {
                if visible.try_get().unwrap_or(false) {
                    let (shown, ()) = create_root(|| ());
                    cleanup(move || shown.dispose());
                    shown.run(&work);
                }
            });
        });
    }

    /// Runs `f` under the session's owner, wherever it is called from (a
    /// view's press handler): what it makes, a new subscription with
    /// `release_on_cleanup` say, lives as long as the session rather than
    /// the view.
    pub fn run<R>(&self, f: impl FnOnce() -> R) -> R {
        self.owner.run(f)
    }
}
