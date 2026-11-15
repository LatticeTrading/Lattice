//! Who asked for a piece of data: what the connection panel names beside
//! each topic and request.
//!
//! Demand is registered in three places (a socket subscription, a query
//! watch, a feed lease) and each notes who asked without its caller saying:
//!
//! - A widget's session provides its [`Consumer`] as context, so whatever
//!   is asked for under the session's owner is the widget's.
//! - A shared feed asks for its own data (a topic) on behalf of whoever
//!   leases it. While it starts, what it asks for is held by its leases'
//!   holders ([`Consumers::scope`]), followed as they come and go: the
//!   topic a second widget joins later names both widgets.
//! - A part of the app that is no widget names itself around what it asks
//!   for ([`Consumer::scope`]).
//!
//! A scope holds what is asked for directly inside it, under the owner it
//! was entered in. An effect that something inside it sets off runs under
//! its own owner, and what that asks for is its own widget's.
//!
//! Nothing is kept per message, and nothing is resolved until the panel
//! asks ([`Held::of`]).

use std::{cell::RefCell, rc::Rc};

use nsh_core::prelude::*;

/// Who wants a piece of data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Consumer {
    /// A widget, by its session's number.
    Widget(u64),
    /// A part of the app that is not a widget, by name (`Market directory`).
    Part(&'static str),
}

impl Consumer {
    /// Runs `f` with what it asks for (subscriptions, watches, leases) held
    /// by this consumer, whatever its owner's context names. Enter it under
    /// the owner the asking runs under (inside an `Owner::run`, not around
    /// it).
    pub fn scope<R>(self, f: impl FnOnce() -> R) -> R {
        scoped(Holder::One(self), f)
    }
}

thread_local! {
    /// The scopes entered and not left, innermost last: who holds what is
    /// asked for, and the owner each was entered under.
    static SCOPE: RefCell<Vec<(Owner, Holder)>> = const { RefCell::new(Vec::new()) };
}

/// Runs `f` with `holder` holding what it asks for under the current
/// owner.
fn scoped<R>(holder: Holder, f: impl FnOnce() -> R) -> R {
    /// Ends the scope, on a panic in `f` too.
    struct Leave;
    impl Drop for Leave {
        fn drop(&mut self) {
            SCOPE.with(|scope| scope.borrow_mut().pop());
        }
    }
    SCOPE.with(|scope| scope.borrow_mut().push((Owner::current(), holder)));
    let _leave = Leave;
    f()
}

/// Who holds one registration of demand.
#[derive(Clone, Default)]
pub(crate) enum Holder {
    /// Nobody said: it was asked for outside a session and a scope.
    #[default]
    Unknown,
    One(Consumer),
    /// Whoever holds a shared feed, as of when it is asked.
    Shared(Consumers),
}

impl Holder {
    /// Who asks right now: the innermost scope's holder while still under
    /// the owner it was entered in, else the consumer the current owner's
    /// context names (a widget's session).
    pub(crate) fn current() -> Self {
        let owner = Owner::current();
        SCOPE
            .with(|scope| match scope.borrow().last() {
                Some((entered, holder)) if *entered == owner => Some(holder.clone()),
                _ => None,
            })
            .or_else(|| use_context::<Consumer>().map(Self::One))
            .unwrap_or_default()
    }

    fn collect(&self, held: &mut Held, depth: u8) {
        match self {
            Self::Unknown => held.unattributed += 1,
            Self::One(consumer) => {
                if !held.consumers.contains(consumer) {
                    held.consumers.push(*consumer);
                }
            }
            Self::Shared(consumers) => consumers.collect(held, depth),
        }
    }
}

/// How deep feeds leasing feeds are followed (a pair leases two books).
const MAX_DEPTH: u8 = 8;

/// The holders of something shared (a feed), in the order they came. Cloning
/// is cheap: every clone is the same holders.
#[derive(Clone, Default)]
pub(crate) struct Consumers(Rc<RefCell<Vec<(u64, Holder)>>>);

impl Consumers {
    /// `holder` holds it too, until [`Self::remove`] of `id`.
    pub(crate) fn add(&self, id: u64, holder: Holder) {
        self.0.borrow_mut().push((id, holder));
    }

    pub(crate) fn remove(&self, id: u64) {
        self.0.borrow_mut().retain(|(held, _)| *held != id);
    }

    /// Runs `f` with what it asks for held by these holders.
    pub(crate) fn scope<R>(&self, f: impl FnOnce() -> R) -> R {
        scoped(Holder::Shared(self.clone()), f)
    }

    fn collect(&self, held: &mut Held, depth: u8) {
        if depth >= MAX_DEPTH {
            return;
        }
        for (_, holder) in self.0.borrow().iter() {
            holder.collect(held, depth + 1);
        }
    }
}

/// Who holds one topic or request, as of now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Held {
    /// Each consumer once, in the order they came.
    pub consumers: Vec<Consumer>,
    /// How many holders nobody named.
    pub unattributed: usize,
}

impl Held {
    /// The consumers behind `holders`.
    pub(crate) fn of<'a>(holders: impl IntoIterator<Item = &'a Holder>) -> Self {
        let mut held = Self::default();
        for holder in holders {
            holder.collect(&mut held, 0);
        }
        held
    }
}
