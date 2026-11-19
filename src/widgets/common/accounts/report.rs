//! A report of each of the accounts a widget's settings name, kept in step
//! with them ([`follow_all`], [`Reports`]), and where they stand as one
//! ([`Standing`]).
//!
//! The session holds the watches: they are made under the session's owner,
//! so the accounts are polled for as long as the widget is in the
//! workspace, shown or not, and showing it again is instant. An account
//! that stays keeps its watch as others come and go, and a new one is
//! watched before those no longer named are let go. Every widget watching
//! one account shares one request (the source's hub does that).
//!
//! Several accounts stand as one ([`together`]): loading until each has
//! either arrived or failed, so that a sum never shows with an account still
//! on its way; then ready with all of them, and with only some saying which
//! are missing and why ([`Standing::Partial`]) over what the others add up
//! to. One account stands as itself.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use nsh_core::prelude::*;

use crate::{
    data::{
        SessionScope,
        net::{QueryState, Watch},
        portfolio::{Account, Unavailable},
    },
    exchanges,
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
    /// A report shows of some of the accounts a widget shows together, and
    /// not of the others: which, and why (`Lighter 0x1234…cdef: offline`).
    Partial(String),
}

impl Standing {
    /// Where a watch whose state is `state` stands.
    fn of<T>(state: &QueryState<T>) -> Self {
        match (&state.data, &state.error) {
            (None, None) => Self::Loading,
            (None, Some(error)) => Self::Failed(error.message.clone()),
            (Some(_), None) => Self::Ready,
            (Some(_), Some(error)) => Self::Stale(error.message.clone()),
        }
    }
}

/// What the watch of one account gives: its state, or why there is none.
type Made<T> = Result<ReadSignal<QueryState<T>>, Unavailable>;

/// The watches held of several accounts: each account's, with what it
/// gives (none of an account that cannot be watched).
type Held<T> = HashMap<Account, (Made<T>, Option<Watch<T>>)>;

/// One of the accounts a widget shows together: its latest report, and
/// where it stands.
struct Part<T> {
    account: Account,
    /// Its latest report, kept through later failures.
    data: Option<Rc<T>>,
    standing: Standing,
}

impl<T> Clone for Part<T> {
    fn clone(&self) -> Self {
        Self {
            account: self.account.clone(),
            data: self.data.clone(),
            standing: self.standing.clone(),
        }
    }
}

/// The same account, standing the same, with the very same report.
impl<T> PartialEq for Part<T> {
    fn eq(&self, other: &Self) -> bool {
        let same = match (&self.data, &other.data) {
            (None, None) => true,
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            _ => false,
        };
        same && self.account == other.account && self.standing == other.standing
    }
}

/// A report of each of the accounts a widget's settings name.
pub(crate) struct Reports<T: 'static> {
    /// Where they stand as one.
    pub standing: Memo<Standing>,
    parts: Memo<Vec<Part<T>>>,
}

impl<T> Clone for Reports<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Reports<T> {}

impl<T> Reports<T> {
    /// The reports that have arrived, each with its account, in the
    /// settings' order, once every account's has arrived or failed
    /// (tracked): what the widget shows and adds up. `None` while one is
    /// still on its way, and when none has arrived.
    pub fn arrived(&self) -> Option<Vec<(Account, Rc<T>)>> {
        self.parts.with(|parts| {
            if parts.iter().any(|part| part.standing == Standing::Loading) {
                return None;
            }
            let arrived: Vec<(Account, Rc<T>)> = parts
                .iter()
                .filter_map(|part| Some((part.account.clone(), part.data.clone()?)))
                .collect();
            (!arrived.is_empty()).then_some(arrived)
        })
    }

    /// Whether a report shows: every account's has arrived or failed, and
    /// one at least has arrived (tracked).
    pub fn shown(&self) -> bool {
        matches!(
            self.standing.get(),
            Standing::Ready | Standing::Stale(_) | Standing::Partial(_)
        )
    }
}

/// What a note calls `account`: its exchange and its address in short.
fn named(account: &Account) -> String {
    format!(
        "{} {}",
        exchanges::display_name(&account.venue),
        account.short_address()
    )
}

/// Why an account standing as `standing` has no report to show, in a note
/// that names it; `None` while it has one, or may yet.
fn missing(standing: &Standing) -> Option<String> {
    Some(match standing {
        Standing::Unavailable(Unavailable::UnknownVenue) => {
            "its exchange is not available in this version of Lattice".to_owned()
        }
        Standing::Unavailable(Unavailable::NotReported) => {
            "its exchange does not report this".to_owned()
        }
        Standing::Failed(why) => why.clone(),
        _ => return None,
    })
}

/// Where accounts standing as `parts` say stand as one.
pub(super) fn together(parts: &[(&Account, &Standing)]) -> Standing {
    if let [(_, only)] = parts {
        return (*only).clone();
    }
    if parts.is_empty() {
        return Standing::Unset;
    }
    if parts
        .iter()
        .any(|(_, standing)| **standing == Standing::Loading)
    {
        return Standing::Loading;
    }
    let absent: Vec<String> = parts
        .iter()
        .filter_map(|(account, standing)| {
            Some(format!("{}: {}", named(account), missing(standing)?))
        })
        .collect();
    if absent.len() == parts.len() {
        // Nothing to show. Accounts of one exchange that cannot be asked
        // for one reason are said as one account would be; anything else
        // names each.
        let (first, standing) = parts[0];
        let alike = parts
            .iter()
            .all(|(account, other)| account.venue == first.venue && *other == standing);
        return match standing {
            Standing::Unavailable(unavailable) if alike => Standing::Unavailable(*unavailable),
            _ => Standing::Failed(absent.join("; ")),
        };
    }
    if !absent.is_empty() {
        return Standing::Partial(absent.join("; "));
    }
    let stale = parts.iter().find_map(|(account, standing)| match standing {
        Standing::Stale(why) => Some(format!("{}: {why}", named(account))),
        _ => None,
    });
    stale.map_or(Standing::Ready, Standing::Stale)
}

/// The report `watch` makes of each of `accounts`, followed as they
/// change: an account that stays keeps its watch, a new one is watched
/// before those no longer named are let go. Call it under the session's
/// owner (in the widget's session); `accounts` is read from the session's
/// copy of the settings, so the reports follow them while no view shows the
/// widget.
pub(crate) fn follow_all<T: 'static>(
    scope: SessionScope,
    accounts: Memo<Vec<Account>>,
    watch: impl Fn(&Account) -> Result<Watch<T>, Unavailable> + 'static,
) -> Reports<T> {
    let current = signal(Vec::<(Account, Made<T>)>::new());
    let held: Rc<RefCell<Held<T>>> = Rc::default();
    effect(move || {
        let accounts = accounts.get();
        untracked(|| {
            let mut previous = std::mem::take(&mut *held.borrow_mut());
            let mut kept = HashMap::with_capacity(accounts.len());
            let mut next = Vec::with_capacity(accounts.len());
            for account in accounts {
                // Under the session, not this run of the effect, which the
                // watch's state must outlive.
                let (made, watch) = previous.remove(&account).unwrap_or_else(|| {
                    match scope.run(|| watch(&account)) {
                        Ok(watch) => (Ok(watch.state()), Some(watch)),
                        Err(unavailable) => (Err(unavailable), None),
                    }
                });
                next.push((account.clone(), made));
                kept.insert(account, (made, watch));
            }
            // The new watches are held before the old ones go.
            *held.borrow_mut() = kept;
            current.set(next);
            drop(previous);
        });
    });
    let parts = computed(move || {
        current.with(|current| {
            current
                .iter()
                .map(|(account, made)| {
                    let (data, standing) = match made {
                        Err(unavailable) => (None, Standing::Unavailable(*unavailable)),
                        Ok(state) => state.with(|state| (state.data.clone(), Standing::of(state))),
                    };
                    Part {
                        account: account.clone(),
                        data,
                        standing,
                    }
                })
                .collect::<Vec<_>>()
        })
    });
    let standing = computed(move || {
        parts.with(|parts| {
            let parts: Vec<(&Account, &Standing)> = parts
                .iter()
                .map(|part| (&part.account, &part.standing))
                .collect();
            together(&parts)
        })
    });
    Reports { standing, parts }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn several_accounts_stand_as_one() {
        let account = |venue: &str| Account {
            venue: venue.to_owned(),
            address: "0x0123456789abcdef0123456789abcdef01234567".to_owned(),
        };
        let (first, second) = (account("hyperliquid"), account("lighterxyz"));
        let of = |a: Standing, b: Standing| together(&[(&first, &a), (&second, &b)]);
        let failed = || Standing::Failed("offline".to_owned());
        let unknown = Standing::Unavailable(Unavailable::UnknownVenue);

        // None, and one alone, which stands as itself.
        assert_eq!(together(&[]), Standing::Unset);
        assert_eq!(together(&[(&first, &failed())]), failed());
        // Loading until each has arrived or failed.
        assert_eq!(of(Standing::Ready, Standing::Ready), Standing::Ready);
        assert_eq!(of(Standing::Ready, Standing::Loading), Standing::Loading);
        assert_eq!(of(failed(), Standing::Loading), Standing::Loading);
        // Some of them: which are missing, and why.
        assert_eq!(
            of(Standing::Ready, failed()),
            Standing::Partial("Lighter 0x0123…4567: offline".to_owned())
        );
        let partial = of(unknown.clone(), Standing::Stale("slow".to_owned()));
        assert!(
            matches!(&partial, Standing::Partial(why) if why.starts_with("Hyperliquid 0x0123…4567: its exchange")),
            "{partial:?}"
        );
        // All of them, one not up to date.
        assert_eq!(
            of(Standing::Stale("slow".to_owned()), Standing::Ready),
            Standing::Stale("Hyperliquid 0x0123…4567: slow".to_owned())
        );
        // None of them: failed when one was asked for, else unavailable.
        assert_eq!(
            of(failed(), failed()),
            Standing::Failed(
                "Hyperliquid 0x0123…4567: offline; Lighter 0x0123…4567: offline".to_owned()
            )
        );
        assert!(matches!(of(unknown.clone(), failed()), Standing::Failed(_)));
        // Two exchanges this build does not know: each is named.
        assert!(matches!(
            of(unknown.clone(), unknown.clone()),
            Standing::Failed(_)
        ));
        // Two accounts of one: said as one would be.
        assert_eq!(together(&[(&first, &unknown), (&first, &unknown)]), unknown);
    }
}
