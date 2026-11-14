//! [`Shared`]: a value shared by reference, equal only to itself.

use std::rc::Rc;

/// A value shared by reference, equal only to itself: what a signal of a
/// big value rebuilt whole (a universe, a table, the rows shown, a
/// listing) holds, so setting it never compares the contents.
pub(crate) struct Shared<T>(pub Rc<T>);

impl<T> Shared<T> {
    pub fn new(value: T) -> Self {
        Self(Rc::new(value))
    }
}

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
