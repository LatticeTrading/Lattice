//! Keeping a consumer's panic out of shared infrastructure.
//!
//! The data layer calls code it does not own: a socket subscriber's
//! callbacks, a query's fetch, a watcher's signal. A panic there must cost
//! that consumer that one delivery, never the socket, the poll loop or the
//! other consumers, so every such call goes through [`contain`] (or a
//! future through [`Contained`]) and the caller logs what it caught with
//! the topic or key it was for.
//!
//! This works where panics unwind: the desktop builds. NSH builds the web
//! runtime with `panic = "abort"` (its generated `web` and `web-dev`
//! profiles), so on wasm32 a panic runs NSH's hook, which shows the error
//! overlay, and the module traps; the page then stops calling into it. No
//! Rust code can survive that, so on the web the only defense is not to
//! panic. See the guide's data layer section.

use std::{
    any::Any,
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    task::{Context, Poll},
};

/// Runs `f`; its panic's message instead of its result when it panicked.
///
/// Unwind safety is asserted: callers run consumer code only while they
/// hold no borrow of their own state, so a panic leaves that state as it
/// was; what the consumer's own state looks like after its panic is its
/// own business.
pub(crate) fn contain<R>(f: impl FnOnce() -> R) -> Result<R, String> {
    catch_unwind(AssertUnwindSafe(f)).map_err(panic_message)
}

/// `future`, ending with its panic's message if a poll of it panics. A
/// future that panicked is never polled again; it is dropped right away,
/// inside the guard too.
pub(crate) fn contained<F: Future>(future: F) -> Contained<F> {
    Contained(Some(Box::pin(future)))
}

/// See [`contained`].
pub(crate) struct Contained<F>(Option<Pin<Box<F>>>);

impl<F: Future> Future for Contained<F> {
    type Output = Result<F::Output, String>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let Some(future) = self.0.as_mut() else {
            return Poll::Ready(Err("polled after it panicked".to_owned()));
        };
        match contain(|| future.as_mut().poll(cx)) {
            Ok(poll) => poll.map(Ok),
            Err(message) => {
                let broken = self.0.take();
                // Its drop is consumer code as well.
                let _ = contain(move || drop(broken));
                Poll::Ready(Err(message))
            }
        }
    }
}

/// The text a panic was raised with.
fn panic_message(payload: Box<dyn Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => match payload.downcast::<&'static str>() {
            Ok(message) => (*message).to_owned(),
            Err(_) => "a panic without a message".to_owned(),
        },
    }
}
