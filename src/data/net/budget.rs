//! A request budget: what an API lets one client ask of it in a window of
//! time, kept on this side so its limit is not met on the wire.
//!
//! An API the app asks directly (an exchange's own, rather than Lattice's
//! backend) limits each client, usually by IP address and by weight: every
//! request costs some of an allowance that comes back as time passes
//! (Hyperliquid: 1200 a minute, 2 to 60 a request). The limit is the
//! user's own, so it is kept where the user's requests are made:
//!
//! - An integration spends a request's weight before it sends it
//!   ([`RateBudget::spend`]). While the window has room that takes no time;
//!   otherwise the request waits until enough of what was spent is a window
//!   old. A poll slows down instead of failing, and the API never has to
//!   refuse one.
//! - What an API charges once it has answered (a weight per item returned)
//!   is added after the fact ([`RateBudget::charge`]).
//! - When the API refuses a request all the same (something else asks from
//!   the same address), the budget is spent whole
//!   ([`RateBudget::exhaust`]): nothing more is sent for a window.
//!
//! The window slides: a spend counts for one window from when it was made,
//! which never lets more through in any window than a fixed one would.

use std::{cell::RefCell, collections::VecDeque, rc::Rc, time::Duration};

use nsh_core::{
    prelude::*,
    time::{Instant, now},
};

use super::error::ApiError;

/// One API's request budget. Cloning is cheap and shares it.
#[derive(Clone)]
pub struct RateBudget(Rc<Inner>);

struct Inner {
    capacity: u32,
    window: Duration,
    /// What was spent within the last window, oldest first.
    spent: RefCell<VecDeque<(Instant, u32)>>,
}

impl RateBudget {
    /// A budget of `capacity` weight in any `window`.
    pub fn new(capacity: u32, window: Duration) -> Self {
        Self(Rc::new(Inner {
            capacity,
            window,
            spent: RefCell::new(VecDeque::new()),
        }))
    }

    /// The weight a window allows.
    pub fn capacity(&self) -> u32 {
        self.0.capacity
    }

    /// The weight spent in the window that ends now.
    pub fn used(&self) -> u32 {
        self.used_at(now())
    }

    /// Spends `weight` once the window has room for it, waiting until it
    /// does. A weight no window could hold is refused. Dropping the future
    /// spends nothing.
    pub async fn spend(&self, weight: u32) -> Result<(), ApiError> {
        if weight > self.0.capacity {
            return Err(ApiError::invalid_request(format!(
                "a request of weight {weight} exceeds the budget of {} per {}s",
                self.0.capacity,
                self.0.window.as_secs()
            )));
        }
        loop {
            match self.try_spend_at(weight, now()) {
                Ok(()) => return Ok(()),
                Err(wait) => timer(wait).await,
            }
        }
    }

    /// Adds `weight` the API charged for a request already sent, whether or
    /// not the window has room for it.
    pub fn charge(&self, weight: u32) {
        if weight > 0 {
            self.0.spent.borrow_mut().push_back((now(), weight));
        }
    }

    /// Spends what is left of the window: the API said its limit was
    /// reached, so nothing more goes out until a window has passed.
    pub fn exhaust(&self) {
        let at = now();
        let left = self.0.capacity.saturating_sub(self.used_at(at));
        if left > 0 {
            self.0.spent.borrow_mut().push_back((at, left));
        }
    }

    /// The weight spent in the window that ends at `at`.
    fn used_at(&self, at: Instant) -> u32 {
        let mut spent = self.0.spent.borrow_mut();
        while spent
            .front()
            .is_some_and(|(made, _)| at.saturating_duration_since(*made) >= self.0.window)
        {
            spent.pop_front();
        }
        spent
            .iter()
            .fold(0_u32, |used, (_, weight)| used.saturating_add(*weight))
    }

    /// Spends `weight` at `at` when the window has room for it; else how
    /// long until it has.
    fn try_spend_at(&self, weight: u32, at: Instant) -> Result<(), Duration> {
        let used = self.used_at(at);
        let mut spent = self.0.spent.borrow_mut();
        let over = used.saturating_add(weight).saturating_sub(self.0.capacity);
        if over == 0 {
            spent.push_back((at, weight));
            return Ok(());
        }
        // Room comes back as the oldest spends leave the window: wait for
        // the one whose leaving frees enough.
        let mut freed = 0_u32;
        for (made, weight) in spent.iter() {
            freed = freed.saturating_add(*weight);
            if freed >= over {
                let frees = *made + self.0.window;
                return Err(frees.saturating_duration_since(at));
            }
        }
        Err(self.0.window)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: Duration = Duration::from_secs(60);

    fn secs(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    #[test]
    fn a_window_takes_its_capacity_and_says_when_it_has_room_again() {
        let budget = RateBudget::new(100, MINUTE);
        let start = Instant::now();
        assert_eq!(budget.try_spend_at(60, start), Ok(()));
        assert_eq!(budget.try_spend_at(40, start + secs(10)), Ok(()));
        assert_eq!(budget.used_at(start + secs(10)), 100);

        // Full: 20 fits once the first spend is a window old, 50 too, and
        // 70 only once the second is.
        let at = start + secs(20);
        assert_eq!(budget.try_spend_at(20, at), Err(secs(40)));
        assert_eq!(budget.try_spend_at(50, at), Err(secs(40)));
        assert_eq!(budget.try_spend_at(70, at), Err(secs(50)));
        assert_eq!(budget.used_at(at), 100, "a refused spend spends nothing");

        // A window after the first spend its weight is back.
        let later = start + MINUTE;
        assert_eq!(budget.used_at(later), 40);
        assert_eq!(budget.try_spend_at(60, later), Ok(()));
        assert_eq!(budget.try_spend_at(1, later), Err(secs(10)));
    }

    #[test]
    fn charges_and_a_refusal_fill_the_window() {
        let budget = RateBudget::new(100, MINUTE);
        assert_eq!(budget.try_spend_at(10, now()), Ok(()));
        // What the API charged afterwards counts, past the capacity too.
        budget.charge(95);
        assert_eq!(budget.used(), 105);
        assert!(budget.try_spend_at(1, now()).is_err());
        budget.exhaust();
        assert_eq!(budget.used(), 105, "nothing is left to spend");

        let refused = RateBudget::new(100, MINUTE);
        assert_eq!(refused.try_spend_at(30, now()), Ok(()));
        refused.exhaust();
        assert_eq!(refused.used(), 100);
        let wait = refused.try_spend_at(2, now()).unwrap_err();
        assert!(wait > secs(58) && wait <= MINUTE, "{wait:?}");
    }
}
