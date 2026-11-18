//! One report of the account a widget's settings name, kept in step with
//! them ([`follow`]), and where it stands ([`Standing`]).
//!
//! The session holds the watch: it is made under the session's owner, so
//! the account is polled for as long as the widget is in the workspace,
//! shown or not, and showing it again is instant. Another account is
//! watched before the last one is let go. Every widget watching one account
//! shares one request (the source's hub does that).

use std::{cell::RefCell, rc::Rc};

use nsh_core::prelude::*;

use crate::data::{
    SessionScope,
    net::{QueryState, Watch},
    portfolio::{Account, Unavailable},
};

/// Where a report of a widget's account stands: what the widget says
/// instead of, or above, its figures.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Standing {
    /// The settings name no account.
    Unset,
    /// The report cannot be asked for, and why.
    Unavailable(Unavailable),
    /// Nothing has arrived yet.
    Loading,
    /// Nothing has arrived, and the latest request failed: why.
    Failed(String),
    /// A report shows.
    Ready,
    /// A report shows, and the latest request for a newer one failed: why.
    Stale(String),
}

/// What the watch of the account in the settings gives: its state, or why
/// there is no watch; `None` with no account.
type Current<T> = Option<Result<ReadSignal<QueryState<T>>, Unavailable>>;

/// A report of the account a widget's settings name.
pub(crate) struct Report<T: 'static> {
    /// Where it stands.
    pub standing: Memo<Standing>,
    state: Memo<Option<QueryState<T>>>,
}

impl<T> Clone for Report<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Report<T> {}

impl<T> Report<T> {
    /// The latest report, kept through later failures (tracked); `None`
    /// until one arrives, and with no account.
    pub fn data(&self) -> Option<Rc<T>> {
        self.state.with(|state| state.as_ref()?.data.clone())
    }
}

/// The report `watch` makes of `account`, followed as the account changes.
/// Call it under the session's owner (in the widget's session); `account`
/// is read from the session's copy of the settings, so the report follows
/// them while no view shows the widget.
pub(crate) fn follow<T: 'static>(
    scope: SessionScope,
    account: Memo<Option<Account>>,
    watch: impl Fn(&Account) -> Result<Watch<T>, Unavailable> + 'static,
) -> Report<T> {
    let current = signal(Current::<T>::None);
    let held: Rc<RefCell<Option<Watch<T>>>> = Rc::default();
    effect(move || {
        let account = account.get();
        untracked(|| {
            // Under the session, not this run of the effect, which the
            // watch's state must outlive.
            let made = account.map(|account| scope.run(|| watch(&account)));
            let (next, watch) = match made {
                None => (None, None),
                Some(Ok(watch)) => (Some(Ok(watch.state())), Some(watch)),
                Some(Err(unavailable)) => (Some(Err(unavailable)), None),
            };
            // The new watch is held before the old one goes.
            let previous = std::mem::replace(&mut *held.borrow_mut(), watch);
            current.set(next);
            drop(previous);
        });
    });
    let state = computed(move || match current.get() {
        Some(Ok(state)) => Some(state.get()),
        _ => None,
    });
    let standing = computed(move || match current.get() {
        None => Standing::Unset,
        Some(Err(unavailable)) => Standing::Unavailable(unavailable),
        Some(Ok(state)) => state.with(|state| match (&state.data, &state.error) {
            (None, None) => Standing::Loading,
            (None, Some(error)) => Standing::Failed(error.message.clone()),
            (Some(_), None) => Standing::Ready,
            (Some(_), Some(error)) => Standing::Stale(error.message.clone()),
        }),
    });
    Report { standing, state }
}
