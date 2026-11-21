//! Async helpers on the engine's executor.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    future::{Future, poll_fn},
    pin::pin,
    rc::Rc,
    task::Poll,
    time::Duration,
};

use nsh_core::prelude::*;

/// A task on the engine thread that ends when this handle drops.
///
/// NSH's `spawn` ties a task to the current owner, which in a callback is
/// whatever widget happened to run it. Services start their tasks with this
/// instead, so a task lives exactly as long as the service state holding
/// its handle. Dropping the handle from inside the task itself is fine: the
/// executor finishes the poll, then drops the future.
pub(crate) struct TaskHandle(Option<Spawned<()>>);

impl TaskHandle {
    pub(crate) fn spawn(future: impl Future<Output = ()> + 'static) -> Self {
        // No owner: nothing else ends it, and no cleanup piles up on a
        // long-lived owner for every short task.
        Self(Some(Owner::detached().run(|| spawn(future))))
    }
}

impl Drop for TaskHandle {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.cancel();
        }
    }
}

/// Tasks that run side by side, each forgotten when it ends (the requests a
/// wallet is answering, the connections a server has open). Dropping the
/// last clone of the set ends those still running.
#[derive(Clone, Default)]
pub(crate) struct TaskSet(Rc<RefCell<Running>>);

#[derive(Default)]
struct Running {
    last: u64,
    tasks: HashMap<u64, TaskHandle>,
}

impl TaskSet {
    /// Runs `future` as a task of the set.
    pub(crate) fn spawn(&self, future: impl Future<Output = ()> + 'static) {
        let id = {
            let mut running = self.0.borrow_mut();
            running.last += 1;
            running.last
        };
        // Held weakly: a task must not keep alive the set that holds it.
        let set = Rc::downgrade(&self.0);
        let done = Rc::new(Cell::new(false));
        let task = TaskHandle::spawn({
            let done = done.clone();
            async move {
                future.await;
                done.set(true);
                // Its own handle, which is fine to drop from here.
                let finished = set
                    .upgrade()
                    .and_then(|set| set.borrow_mut().tasks.remove(&id));
                drop(finished);
            }
        });
        // One that ended within its first poll has nothing to forget.
        if !done.get() {
            self.0.borrow_mut().tasks.insert(id, task);
        }
    }
}

/// `future`'s output, or `None` once `limit` passes first. Uses the engine's
/// timer, so a test on a manual clock controls it too.
pub(crate) async fn deadline<F: Future>(limit: Duration, future: F) -> Option<F::Output> {
    let mut future = pin!(future);
    let mut expired = pin!(timer(limit));
    poll_fn(|cx| {
        if let Poll::Ready(output) = future.as_mut().poll(cx) {
            return Poll::Ready(Some(output));
        }
        expired.as_mut().poll(cx).map(|()| None)
    })
    .await
}
