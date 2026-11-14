//! [`Keyed`]: values held one per key (watches, leases), kept in step
//! with what is wanted.

use std::cell::RefCell;

/// Values held one per key, in the order they were last wanted.
/// [`follow`](Self::follow) keeps the ones still wanted, makes the new
/// ones and lets the rest go.
pub(crate) struct Keyed<K, V>(RefCell<Vec<(K, V)>>);

impl<K, V> Default for Keyed<K, V> {
    fn default() -> Self {
        Self(RefCell::default())
    }
}

impl<K: PartialEq, V> Keyed<K, V> {
    /// Holds one value per item of `wanted` (its key, and what its value
    /// is made from), in `wanted`'s order:
    ///
    /// - a key already held keeps its value when `keep(value, item)` says
    ///   so (it may update the value in place); otherwise a new value is
    ///   made before the old one goes, so what they share stays cached
    ///   between them;
    /// - a new key gets `make(item)`;
    /// - the values no longer wanted are let go once the new ones are held.
    ///
    /// `keep` and `make` must not use `self`: it is empty while they run.
    pub fn follow<T>(
        &self,
        wanted: impl IntoIterator<Item = (K, T)>,
        mut keep: impl FnMut(&mut V, &T) -> bool,
        mut make: impl FnMut(&T) -> V,
    ) {
        let mut previous = std::mem::take(&mut *self.0.borrow_mut());
        let mut next = Vec::new();
        for (key, item) in wanted {
            let held = previous
                .iter()
                .position(|(held, _)| *held == key)
                .map(|at| previous.swap_remove(at).1);
            let value = match held {
                Some(mut value) => {
                    if keep(&mut value, &item) {
                        value
                    } else {
                        let made = make(&item);
                        drop(value);
                        made
                    }
                }
                None => make(&item),
            };
            next.push((key, value));
        }
        *self.0.borrow_mut() = next;
        // What is no longer wanted: its demand ends here.
        drop(previous);
    }

    /// Calls `f` with the keys and values held, in order.
    pub fn with<R>(&self, f: impl FnOnce(&[(K, V)]) -> R) -> R {
        f(&self.0.borrow())
    }
}
