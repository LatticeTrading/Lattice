//! Price pairs: two markets' mid prices, sampled together each time either
//! of them moves, shared by every widget charting the pair (the old app's
//! `PremiumArbSession` and its shared pair).
//!
//! A [`PricePairFeed`] is a lease on the feed of one ordered pair of
//! markets. The first lease starts it: it takes a lease on each market's
//! book at one level a side ([`Ferris::order_book`], so it shares those
//! topics with whatever else reads them) and keeps each one's mid price.
//! Every change of either book makes a [`PairSample`] of both prices, once
//! both are known; the newest [`MAX_SAMPLES`] are kept. Later leases of the
//! same two markets in the same order share it: one history, whichever
//! widget asked first. The last lease dropping ends it and lets go of both
//! books. The sharing, leases and listeners are the generic
//! [`FeedHub`](crate::data::net::FeedHub)'s; this module is the pairing
//! only.
//!
//! - A book's mid price is halfway between its best bid and its best ask,
//!   or the one it has while a side is empty ([`OrderBook::mid_price`]).
//! - A change that brings what was sampled before (the same two prices from
//!   books of the same two times: a snapshot replayed, a status change)
//!   makes no sample.
//! - Samples are in time order, each later than the one before: one whose
//!   book is no newer than the last sample is put a tenth of a millisecond
//!   after it.
//!
//! Each lease's `on_change` hears every new sample and every change of
//! [`PricePairFeed::failed`]; it runs in the socket's callback, so a
//! consumer marks itself dirty there and does its work once a frame.

use std::{
    cell::{Cell, RefCell},
    collections::{HashSet, VecDeque},
    rc::Rc,
};

use nsh_core::time::unix_ms;

use super::{BookTraits, Ferris, MarketKey, MarketSelection, OrderBook, OrderBookFeed};
use crate::data::net::{FeedLease, FeedNode};

/// The samples a pair keeps (the old `MAX_PREMIUM_ARB_POINTS`).
pub const MAX_SAMPLES: usize = 12_000;
/// How long after the sample before it one is put whose book is no newer,
/// in seconds.
const STEP: f64 = 0.0001;
/// The pair's markets, as its quotes and books are numbered.
const PRIMARY: usize = 0;
const SECONDARY: usize = 1;

/// Both markets' mid prices at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PairSample {
    /// When, in Unix seconds: later than the sample before it.
    pub time: f64,
    pub primary: f64,
    pub secondary: f64,
}

/// One market's mid price, and when its book was made (Unix milliseconds).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Quote {
    price: f64,
    timestamp: f64,
}

/// A pair's quotes, and the samples made of them.
#[derive(Default)]
struct Samples {
    /// Each market's quote; none until its book has a price.
    quotes: [Option<Quote>; 2],
    /// The newest samples, oldest first.
    kept: VecDeque<PairSample>,
    /// How many samples were ever made.
    made: u64,
    /// What the newest samples were made of (both books' times and both
    /// prices), so the same quotes are sampled once; `order` is the same,
    /// oldest first.
    seen: HashSet<[u64; 4]>,
    order: VecDeque<[u64; 4]>,
    /// The newest sample's time.
    last_time: f64,
}

impl Samples {
    /// Takes `book` as the one `leg` has now.
    fn quote(&mut self, leg: usize, book: Option<&OrderBook>) {
        self.quotes[leg] = book.and_then(|book| {
            Some(Quote {
                price: book.mid_price()?,
                timestamp: book.timestamp,
            })
        });
    }

    /// Makes a sample of both prices as of `at` (Unix milliseconds; `now`
    /// when it is no time), unless one is unknown or these quotes were
    /// sampled before. Says whether it made one.
    fn sample(&mut self, at: f64, now: f64) -> bool {
        let [Some(primary), Some(secondary)] = self.quotes else {
            return false;
        };
        if !(primary.price.is_finite() && secondary.price.is_finite()) {
            return false;
        }
        let key = [
            primary.timestamp,
            secondary.timestamp,
            primary.price,
            secondary.price,
        ]
        .map(f64::to_bits);
        if !self.seen.insert(key) {
            return false;
        }
        self.order.push_back(key);
        if self.order.len() > MAX_SAMPLES
            && let Some(oldest) = self.order.pop_front()
        {
            self.seen.remove(&oldest);
        }

        let at = if at.is_finite() && at > 0.0 { at } else { now };
        let time = match at / 1_000.0 {
            time if time <= self.last_time => self.last_time + STEP,
            time => time,
        };
        self.last_time = time;
        self.kept.push_back(PairSample {
            time,
            primary: primary.price,
            secondary: secondary.price,
        });
        if self.kept.len() > MAX_SAMPLES {
            self.kept.pop_front();
        }
        self.made += 1;
        true
    }
}

/// A pair's feed, shared by its leases.
type Feed = FeedNode<(MarketKey, MarketKey), Pair>;

/// What a feed keeps: its samples, and its leases on the two books.
#[derive(Default)]
struct Pair {
    samples: RefCell<Samples>,
    /// The books, the primary's first; none until the feed has started.
    books: RefCell<Option<[OrderBookFeed; 2]>>,
    /// Whether either book's topic was refused, as the leases last heard.
    failed: Cell<bool>,
}

/// Starts a new feed: leases both markets' books at one level a side, and
/// samples what they already hold (books another widget streams).
fn start(ferris: &Ferris, feed: &Rc<Feed>, markets: [&MarketSelection; 2]) {
    let lease = |leg: usize| {
        let weak = Rc::downgrade(feed);
        ferris.order_book(markets[leg], 1, move || {
            if let Some(feed) = weak.upgrade() {
                receive(&feed, leg);
            }
        })
    };
    let books = [lease(PRIMARY), lease(SECONDARY)];
    let pair = feed.state();
    {
        let mut samples = pair.samples.borrow_mut();
        let mut newest = 0.0_f64;
        for (leg, feed) in books.iter().enumerate() {
            let book = feed.book();
            samples.quote(leg, book.as_deref());
            newest = newest.max(book.map_or(0.0, |book| book.timestamp));
        }
        samples.sample(newest, unix_ms());
    }
    pair.failed.set(books.iter().any(OrderBookFeed::failed));
    *pair.books.borrow_mut() = Some(books);
}

/// Hears that `leg`'s book or status changed: takes both books as they
/// stand (one market paired with itself is one book, which moves both
/// prices at once) and samples them as of that book's time.
fn receive(feed: &Feed, leg: usize) {
    let pair = feed.state();
    let (sampled, failed) = {
        let books = pair.books.borrow();
        let Some(books) = books.as_ref() else {
            return;
        };
        let mut samples = pair.samples.borrow_mut();
        for (leg, feed) in books.iter().enumerate() {
            samples.quote(leg, feed.book().as_deref());
        }
        let sampled = books[leg]
            .book()
            .is_some_and(|book| samples.sample(book.timestamp, unix_ms()));
        (sampled, books.iter().any(OrderBookFeed::failed))
    };
    let changed = pair.failed.replace(failed) != failed;
    if sampled || changed {
        feed.notify();
    }
}

impl Ferris {
    /// A lease on the paired mid prices of `primary` and `secondary`: their
    /// shared feed (by both markets' [`MarketSelection::identity`], in this
    /// order), started if no lease holds it. `on_change` is told each time
    /// the feed makes a sample or its status changes, until the lease
    /// drops.
    pub fn price_pair(
        &self,
        primary: &MarketSelection,
        secondary: &MarketSelection,
        on_change: impl Fn() + 'static,
    ) -> PricePairFeed {
        let lease = self.feeds.hub::<(MarketKey, MarketKey), Pair>().lease(
            (primary.identity(), secondary.identity()),
            Pair::default,
            |feed| start(self, feed, [primary, secondary]),
            on_change,
        );
        PricePairFeed(lease)
    }
}

/// One consumer's lease on a pair's prices (see [`Ferris::price_pair`]).
/// Dropping it stops its `on_change`, and the last lease ends the feed.
#[must_use = "dropping a price pair feed releases it"]
pub struct PricePairFeed(FeedLease<(MarketKey, MarketKey), Pair>);

impl PricePairFeed {
    /// The samples kept, oldest first, and how many were ever made: the
    /// newest one kept is that number less one, so a consumer that counts
    /// the ones it took finds the ones it has not.
    pub fn with_samples<R>(&self, f: impl FnOnce(&VecDeque<PairSample>, u64) -> R) -> R {
        let samples = self.0.state().samples.borrow();
        f(&samples.kept, samples.made)
    }

    /// The server refused one of its books' topics, or it could not be
    /// asked for.
    pub fn failed(&self) -> bool {
        self.0.state().failed.get()
    }

    /// What its books' newest states say of their amounts and liquidity,
    /// the primary's first; none for a book that has not arrived.
    pub fn traits(&self) -> [Option<BookTraits>; 2] {
        match self.0.state().books.borrow().as_ref() {
            Some(books) => books
                .each_ref()
                .map(|book| Some(book.book()?.traits.clone())),
            None => [None, None],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A book about `price`, a point either side, made at `timestamp`.
    fn book(price: f64, timestamp: f64) -> OrderBook {
        OrderBook::of(&[(price - 1.0, 1.0)], &[(price + 1.0, 1.0)], timestamp)
    }

    const T0: f64 = 1_700_000_000_000.0;

    /// `leg`'s book arriving, as [`receive`] takes it.
    fn arrive(samples: &mut Samples, leg: usize, book: &OrderBook) -> bool {
        samples.quote(leg, Some(book));
        samples.sample(book.timestamp, T0)
    }

    #[test]
    fn a_pair_is_sampled_once_both_prices_are_known_and_never_twice() {
        let mut samples = Samples::default();
        assert!(!arrive(&mut samples, PRIMARY, &book(100.0, T0)));
        assert!(arrive(&mut samples, SECONDARY, &book(110.0, T0)));
        assert_eq!(
            samples.kept,
            [PairSample {
                time: T0 / 1_000.0,
                primary: 100.0,
                secondary: 110.0
            }]
        );
        // The same books again (a snapshot replayed, a status change).
        assert!(!arrive(&mut samples, PRIMARY, &book(100.0, T0)));
        assert!(!arrive(&mut samples, SECONDARY, &book(110.0, T0)));
        assert_eq!((samples.kept.len(), samples.made), (1, 1));
        // A newer book of either is a sample, at that book's time.
        assert!(arrive(&mut samples, PRIMARY, &book(101.0, T0 + 1.0)));
        assert_eq!(
            samples.kept[1],
            PairSample {
                time: (T0 + 1.0) / 1_000.0,
                primary: 101.0,
                secondary: 110.0
            }
        );
        // A book that lost its prices takes its market out of the pair.
        samples.quote(SECONDARY, Some(&OrderBook::default()));
        assert!(!samples.sample(T0 + 2.0, T0));
        assert_eq!(samples.made, 2);
    }

    #[test]
    fn samples_stay_in_time_order_and_only_the_newest_are_kept() {
        let mut samples = Samples::default();
        arrive(&mut samples, PRIMARY, &book(100.0, T0));
        arrive(&mut samples, SECONDARY, &book(110.0, T0));
        for step in 1..=MAX_SAMPLES + 10 {
            let step = step as f64;
            assert!(arrive(
                &mut samples,
                PRIMARY,
                &book(100.0 + step, T0 + step)
            ));
        }
        assert_eq!(samples.kept.len(), MAX_SAMPLES);
        assert_eq!(samples.made, MAX_SAMPLES as u64 + 11);
        assert_eq!(samples.seen.len(), MAX_SAMPLES);
        // The newest book again is no sample.
        let newest = book(110.0 + MAX_SAMPLES as f64, T0 + 10.0 + MAX_SAMPLES as f64);
        assert!(!arrive(&mut samples, PRIMARY, &newest));
        // A book older than the newest sample is put just after it; one
        // with no time is sampled now, which is older too.
        let last = samples.kept[MAX_SAMPLES - 1].time;
        assert!(arrive(&mut samples, SECONDARY, &book(111.0, T0)));
        assert_eq!(samples.kept[MAX_SAMPLES - 1].time, last + STEP);
        assert!(arrive(&mut samples, SECONDARY, &book(112.0, f64::NAN)));
        let times: Vec<f64> = samples.kept.iter().map(|sample| sample.time).collect();
        assert!(times.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn a_mid_price_is_between_the_best_prices_or_the_one_there_is() {
        assert_eq!(book(100.0, T0).mid_price(), Some(100.0));
        let bids_only = OrderBook::of(&[(99.0, 1.0), (98.0, 2.0)], &[], T0);
        assert_eq!(bids_only.mid_price(), Some(99.0));
        let asks_only = OrderBook::of(&[], &[(102.0, 1.0), (101.0, 2.0)], T0);
        assert_eq!(asks_only.mid_price(), Some(101.0));
        assert_eq!(OrderBook::default().mid_price(), None);
    }
}
