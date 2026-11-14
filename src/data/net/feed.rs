//! Shared feeds: one live state per key, started by its first lease and
//! ended by its last. The machinery a streaming service (trades today;
//! order books and candles next) builds on, knowing nothing of what it
//! streams.
//!
//! A service keeps its feeds in a [`FeedHub`], keyed by what tells two
//! apart (a market). [`FeedHub::lease`] gives a lease on the running feed
//! of a key, or makes the feed's state and then starts it (subscribes,
//! spawns its tasks) with the feed in hand, so its callbacks can hold it
//! weakly. Each lease's `on_change` is told every time the service calls
//! [`FeedNode::notify`], until the lease drops. The hub holds feeds weakly:
//! the last lease dropping drops the feed and what it holds (subscriptions,
//! tasks), and the feed leaves the hub.
//!
//! [`FeedHubs`] holds one hub per feed type, so a client offers any number
//! of streaming services from one field.

use std::{
    any::{Any, TypeId},
    cell::{Cell, RefCell},
    collections::HashMap,
    hash::Hash,
    rc::{Rc, Weak},
};

type Listener = Rc<dyn Fn()>;
type Registry<K, S> = RefCell<HashMap<K, Weak<FeedNode<K, S>>>>;

/// One key's feed: the service's state `S`, and its leases' listeners.
pub struct FeedNode<K: Eq + Hash, S> {
    key: K,
    state: S,
    listeners: RefCell<Vec<(u64, Listener)>>,
    next_listener: Cell<u64>,
    hub: Weak<Registry<K, S>>,
}

impl<K: Eq + Hash, S> FeedNode<K, S> {
    /// What the service keeps for this key.
    pub fn state(&self) -> &S {
        &self.state
    }

    /// Tells every lease's `on_change` that something changed, in the
    /// order they leased.
    pub fn notify(&self) {
        let listeners: Vec<Listener> = self
            .listeners
            .borrow()
            .iter()
            .map(|(_, listener)| listener.clone())
            .collect();
        for listener in listeners {
            listener();
        }
    }
}

impl<K: Eq + Hash, S> Drop for FeedNode<K, S> {
    fn drop(&mut self) {
        if let Some(hub) = self.hub.upgrade()
            && let Ok(mut feeds) = hub.try_borrow_mut()
            && feeds
                .get(&self.key)
                .is_some_and(|feed| feed.strong_count() == 0)
        {
            feeds.remove(&self.key);
        }
    }
}

/// One service's feeds, by key. Cloning is cheap: every clone is the same
/// feeds.
pub struct FeedHub<K: Eq + Hash, S>(Rc<Registry<K, S>>);

impl<K: Eq + Hash, S> Clone for FeedHub<K, S> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<K: Eq + Hash, S> Default for FeedHub<K, S> {
    fn default() -> Self {
        Self(Rc::default())
    }
}

impl<K: Eq + Hash + Clone, S> FeedHub<K, S> {
    /// A lease on `key`'s feed. A feed already running is shared; else its
    /// state is `make()`, and `start` runs with the new feed before it is
    /// listed. `on_change` hears [`FeedNode::notify`] until the lease drops.
    pub fn lease(
        &self,
        key: K,
        make: impl FnOnce() -> S,
        start: impl FnOnce(&Rc<FeedNode<K, S>>),
        on_change: impl Fn() + 'static,
    ) -> FeedLease<K, S> {
        let running = self.0.borrow().get(&key).and_then(Weak::upgrade);
        let node = running.unwrap_or_else(|| {
            let node = Rc::new(FeedNode {
                key: key.clone(),
                state: make(),
                listeners: RefCell::new(Vec::new()),
                next_listener: Cell::new(0),
                hub: Rc::downgrade(&self.0),
            });
            start(&node);
            self.0.borrow_mut().insert(key, Rc::downgrade(&node));
            node
        });
        let id = node.next_listener.get();
        node.next_listener.set(id + 1);
        node.listeners.borrow_mut().push((id, Rc::new(on_change)));
        FeedLease { node, id }
    }
}

/// One consumer's lease on a feed (see [`FeedHub::lease`]). Dropping it
/// stops its `on_change`, and the last lease ends the feed.
#[must_use = "dropping a lease releases the feed"]
pub struct FeedLease<K: Eq + Hash, S> {
    node: Rc<FeedNode<K, S>>,
    id: u64,
}

impl<K: Eq + Hash, S> FeedLease<K, S> {
    /// The feed's state.
    pub fn state(&self) -> &S {
        &self.node.state
    }
}

impl<K: Eq + Hash, S> Drop for FeedLease<K, S> {
    fn drop(&mut self) {
        self.node
            .listeners
            .borrow_mut()
            .retain(|(id, _)| *id != self.id);
    }
}

/// One [`FeedHub`] per feed type, made on first use. Cloning is cheap:
/// every clone is the same hubs.
#[derive(Clone, Default)]
pub struct FeedHubs(Rc<RefCell<HashMap<TypeId, Rc<dyn Any>>>>);

impl FeedHubs {
    /// The hub of feeds keyed by `K` holding `S`.
    pub fn hub<K: Eq + Hash + Clone + 'static, S: 'static>(&self) -> FeedHub<K, S> {
        let mut hubs = self.0.borrow_mut();
        let hub = hubs
            .entry(TypeId::of::<FeedHub<K, S>>())
            .or_insert_with(|| Rc::new(FeedHub::<K, S>::default()));
        hub.downcast_ref::<FeedHub<K, S>>()
            .expect("a hub is filed under its own type")
            .clone()
    }
}
