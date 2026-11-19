//! What an address's accounts were worth and made over time
//! ([`Request::Pnl`](super::Request)), as Lighter writes it and as the
//! portfolio reads it ([`Performance`]).
//!
//! Lighter's chart is a list of samples, an hour or a day apart and one
//! more for now. A sample counts, each since the account began: what its
//! perpetuals trading had made (`trade_pnl`, open positions included) and
//! what was moved in and out of it (`inflow`, `outflow`), and the same three
//! for its spot trading (`trade_spot_pnl`, `spot_inflow`, `spot_outflow`),
//! its pool shares (`pool_...`) and its staking (`staking_...`). It also
//! says what was traded since the sample before (`volume`). A member that is
//! nothing may be left out.
//!
//! So a part of the account was worth what went into it, less what came
//! out, with what it made ([`PnlEntry::perps_value`]: the account's
//! collateral with its open positions' profit and loss, which is what
//! Lighter calls its total account value), and the account all four parts
//! together. Lighter says no value outright.
//!
//! The windows ([`History::windows`]) are cut from two requests an account:
//! a week of hours, whose last day is the day, and every day there is,
//! whose last thirty are the month. What a window made is counted from its
//! first sample; over all time it is what Lighter counts, from the
//! account's start. What was traded is the whole account's, so its
//! perpetuals' alone is not reported.
//!
//! An address's accounts are one history ([`performance`]), combined as the
//! portfolio combines accounts ([`Performance::combined`]).

use serde::Deserialize;

use crate::data::portfolio::{Performance, Period, Point, Scope, Window};

/// A day, in seconds.
const DAY: f64 = 86_400.0;
/// An hour, in seconds.
const HOUR: f64 = 3_600.0;
/// How far back the day and the month reach, in seconds.
const DAY_SPAN: f64 = DAY;
const MONTH_SPAN: f64 = 30.0 * DAY;
/// A time above this is in milliseconds, not seconds (Lighter writes a
/// sample's in seconds, and takes either).
const MILLISECONDS_ABOVE: f64 = 100_000_000_000.0;

/// The reply to [`Request::Pnl`](super::Request).
#[derive(Debug, Default, Deserialize)]
pub(super) struct PnlReply {
    #[serde(default)]
    pnl: Option<Vec<PnlEntry>>,
}

impl PnlReply {
    /// Its samples, oldest first, their times in seconds.
    pub fn entries(self) -> Vec<PnlEntry> {
        let mut entries: Vec<PnlEntry> = self
            .pnl
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| entry.timestamp.is_finite() && entry.timestamp > 0.0)
            .map(|entry| PnlEntry {
                timestamp: if entry.timestamp > MILLISECONDS_ABOVE {
                    entry.timestamp / 1000.0
                } else {
                    entry.timestamp
                },
                ..entry
            })
            .collect();
        entries.sort_by(|a, b| a.timestamp.total_cmp(&b.timestamp));
        entries
    }
}

/// One sample of an account: what it had made and moved by then, each
/// since it began, and what it traded since the sample before.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub(super) struct PnlEntry {
    /// When, in Unix seconds.
    timestamp: f64,
    trade_pnl: f64,
    inflow: f64,
    outflow: f64,
    trade_spot_pnl: f64,
    spot_inflow: f64,
    spot_outflow: f64,
    pool_pnl: f64,
    pool_inflow: f64,
    pool_outflow: f64,
    staking_pnl: f64,
    staking_inflow: f64,
    staking_outflow: f64,
    volume: f64,
}

impl PnlEntry {
    fn time_ms(&self) -> f64 {
        self.timestamp * 1000.0
    }

    /// What its perpetuals account was worth: what went in, less what came
    /// out, with what its trading made.
    fn perps_value(&self) -> f64 {
        self.inflow - self.outflow + self.trade_pnl
    }

    fn perps_pnl(&self) -> f64 {
        self.trade_pnl
    }

    /// What the whole account was worth: its perpetuals, its spot
    /// balances, its pool shares and its stake.
    fn total_value(&self) -> f64 {
        self.perps_value()
            + (self.spot_inflow - self.spot_outflow + self.trade_spot_pnl)
            + (self.pool_inflow - self.pool_outflow + self.pool_pnl)
            + (self.staking_inflow - self.staking_outflow + self.staking_pnl)
    }

    fn total_pnl(&self) -> f64 {
        self.trade_pnl + self.trade_spot_pnl + self.pool_pnl + self.staking_pnl
    }
}

/// One account's samples: its last week by the hour and all of its days,
/// each oldest first.
#[derive(Debug, Default)]
pub(super) struct History {
    pub hourly: Vec<PnlEntry>,
    pub daily: Vec<PnlEntry>,
}

impl History {
    /// The windows of the account, as they stand at `now` (Unix seconds).
    fn windows(&self, now: f64) -> Performance {
        let since = |entries: &'_ [PnlEntry], span: f64| {
            let from = now - span;
            let start = entries.partition_point(|entry| entry.timestamp < from);
            start..entries.len()
        };
        let spans = [
            (
                Period::Day,
                &self.hourly[since(&self.hourly, DAY_SPAN)],
                false,
            ),
            (Period::Week, &self.hourly[..], false),
            (
                Period::Month,
                &self.daily[since(&self.daily, MONTH_SPAN)],
                false,
            ),
            (Period::AllTime, &self.daily[..], true),
        ];
        let mut windows = Vec::with_capacity(spans.len() * Scope::ALL.len());
        for scope in Scope::ALL {
            for (period, entries, whole) in spans {
                windows.push(window(period, scope, entries, whole));
            }
        }
        Performance(windows)
    }
}

/// What reads one figure of a sample.
type Read = fn(&PnlEntry) -> f64;

/// The window of `period` and `scope` that `entries` are. `whole` says
/// they are all there is of the account: what it made is then counted from
/// its start, as Lighter counts it, and what it traded includes the first
/// sample's (which otherwise was traded before the window).
fn window(period: Period, scope: Scope, entries: &[PnlEntry], whole: bool) -> Window {
    let (worth, made): (Read, Read) = match scope {
        Scope::Total => (PnlEntry::total_value, PnlEntry::total_pnl),
        Scope::Perps => (PnlEntry::perps_value, PnlEntry::perps_pnl),
    };
    let counted_from = match entries.first() {
        Some(first) if !whole => made(first),
        _ => 0.0,
    };
    let points = |read: &dyn Fn(&PnlEntry) -> f64| -> Vec<Point> {
        entries
            .iter()
            .map(|entry| Point {
                time_ms: entry.time_ms(),
                value: read(entry),
            })
            .collect()
    };
    Window {
        period,
        scope,
        value: points(&worth),
        pnl: points(&|entry| made(entry) - counted_from),
        volume: (scope == Scope::Total).then(|| {
            entries
                .iter()
                .skip(usize::from(!whole))
                .map(|entry| entry.volume)
                .sum()
        }),
    }
}

/// Moves the last sample of each of `series` to the latest of them, when
/// it is less than `step` seconds before it. An account's last sample is
/// of the moment it was asked for, and an address's accounts are asked for
/// one after another: their histories end at one time, not seconds apart
/// with a sample of one account between two of another.
fn align(series: &mut [&mut Vec<PnlEntry>], step: f64) {
    let latest = series
        .iter()
        .filter_map(|entries| entries.last())
        .map(|entry| entry.timestamp)
        .fold(f64::NEG_INFINITY, f64::max);
    for entries in series {
        if let Some(last) = entries.last_mut()
            && latest - last.timestamp < step
        {
            last.timestamp = latest;
        }
    }
}

/// The history of an address whose accounts' are `histories`, as it stands
/// at `now` (Unix seconds): the accounts together. With no account, every
/// window with nothing in it.
pub(super) fn performance(mut histories: Vec<History>, now: f64) -> Performance {
    if histories.is_empty() {
        return History::default().windows(now);
    }
    let mut hourly: Vec<&mut Vec<PnlEntry>> = Vec::with_capacity(histories.len());
    let mut daily: Vec<&mut Vec<PnlEntry>> = Vec::with_capacity(histories.len());
    for history in &mut histories {
        hourly.push(&mut history.hourly);
        daily.push(&mut history.daily);
    }
    align(&mut hourly, HOUR);
    align(&mut daily, DAY);
    let parts: Vec<Performance> = histories
        .iter()
        .map(|history| history.windows(now))
        .collect();
    Performance::combined(&parts)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    /// A time `hours` after an hour's start, in seconds.
    fn at(hours: f64) -> f64 {
        1_794_448_800.0 + hours * HOUR
    }

    fn entries(reply: Value) -> Vec<PnlEntry> {
        serde_json::from_value::<PnlReply>(reply)
            .expect("the documented reply reads")
            .entries()
    }

    #[test]
    fn a_sample_is_worth_what_went_in_less_what_came_out_with_what_it_made() {
        // The whole of a sample as Lighter writes it (an account's, checked
        // against its total account value live, 2026-11-19).
        let read = entries(json!({
            "code": 200,
            "resolution": "1d",
            "pnl": [{
                "timestamp": 1_795_046_400_u64,
                "trade_pnl": 2_453_847.062342, "trade_spot_pnl": -1_380_839.538069,
                "inflow": 10_955_877.452512, "outflow": 12_643_300.991192,
                "spot_outflow": 2_578_195.827292, "spot_inflow": 3_959_035.390412,
                "pool_pnl": 15_452.553404, "pool_inflow": 1_421_737.611908,
                "pool_outflow": 1_437_190.165312,
                "staking_pnl": 1_111_519.126852, "staking_inflow": 1_465_370.548560,
                "staking_outflow": 2_576_889.675412,
                "pool_total_shares": 0, "volume": 21_744_503.497188,
            }],
        }));
        let [sample] = read[..] else {
            panic!("one sample");
        };
        let near = |figure: f64, expected: f64| (figure - expected).abs() < 1e-6;
        assert!(near(sample.perps_value(), 766_423.523662));
        // Its spot, pool shares and stake had all been taken out again.
        assert!(near(sample.total_value(), 766_423.548713));
        assert!(near(sample.perps_pnl(), 2_453_847.062342));
        assert!(near(sample.total_pnl(), 2_199_979.204529));
        assert_eq!(sample.time_ms(), 1_795_046_400_000.0);

        // Members that are nothing may be left out, samples come in any
        // order, and a time in milliseconds is that time.
        let sparse = entries(json!({ "pnl": [
            { "timestamp": 1_795_046_400_000_u64, "inflow": 25, "outflow": 5 },
            { "timestamp": 1_794_960_000_u64, "inflow": 25 },
            { "inflow": 99 },
        ] }));
        assert_eq!(sparse.len(), 2, "a sample with no time is left out");
        assert_eq!(sparse[0].perps_value(), 25.0);
        assert_eq!(
            (sparse[1].timestamp, sparse[1].total_value()),
            (1_795_046_400.0, 20.0)
        );
        // No samples at all.
        assert!(entries(json!({ "code": 200, "pnl": null })).is_empty());
        assert!(entries(json!({ "code": 200 })).is_empty());
    }

    /// An account that put 1000 in, then made `step` an hour (or a day)
    /// and traded 10 between each two samples, sampled at `times`.
    fn climbing(times: &[f64], step: f64) -> Vec<PnlEntry> {
        times
            .iter()
            .enumerate()
            .map(|(n, time)| PnlEntry {
                timestamp: *time,
                inflow: 1_000.0,
                trade_pnl: 50.0 + step * n as f64,
                trade_spot_pnl: 1.0,
                spot_inflow: 100.0,
                volume: 10.0,
                ..PnlEntry::default()
            })
            .collect()
    }

    #[test]
    fn the_windows_are_cut_from_a_week_of_hours_and_every_day() {
        let now = at(168.5);
        let hours: Vec<f64> = (1..=168).map(|hour| at(f64::from(hour))).collect();
        let days: Vec<f64> = (0..40)
            .map(|day| at(168.0) - DAY * f64::from(39 - day))
            .collect();
        let history = History {
            hourly: climbing(&[&hours[..], &[now]].concat(), 2.0),
            daily: climbing(&[&days[..], &[now]].concat(), 5.0),
        };
        let performance = performance(vec![history], now);
        assert_eq!(performance.0.len(), 8);

        // The week is every hour asked for; the day, its last 24 hours
        // (the samples at or after a day ago) and now.
        let week = performance.window(Period::Week, Scope::Total).unwrap();
        assert_eq!(week.value.len(), 169);
        let day = performance.window(Period::Day, Scope::Total).unwrap();
        assert_eq!(day.value.len(), 25);
        assert_eq!(day.value[0].time_ms, at(145.0) * 1000.0);
        assert_eq!(day.value.last().unwrap().time_ms, now * 1000.0);
        // What it is worth is all of it; what the window made is counted
        // from its first sample; what it traded, after that sample.
        assert_eq!(
            day.value[0].value,
            1_000.0 + 100.0 + 50.0 + 1.0 + 2.0 * 144.0
        );
        assert_eq!(day.pnl[0].value, 0.0);
        assert_eq!(day.pnl.last().unwrap().value, 2.0 * 24.0);
        assert_eq!(day.volume, Some(240.0));

        // Its perpetuals alone: without its spot, and with no volume of
        // their own.
        let perps = performance.window(Period::Day, Scope::Perps).unwrap();
        assert_eq!(perps.value[0].value, 1_000.0 + 50.0 + 2.0 * 144.0);
        assert_eq!(perps.pnl.last().unwrap().value, 2.0 * 24.0);
        assert_eq!(perps.volume, None);

        // The month is the last thirty days of all of them; all time is
        // counted as Lighter counts it, from the account's start.
        let month = performance.window(Period::Month, Scope::Total).unwrap();
        assert_eq!(month.value.len(), 31);
        assert_eq!(month.pnl[0].value, 0.0);
        let all = performance.window(Period::AllTime, Scope::Total).unwrap();
        assert_eq!(all.value.len(), 41);
        assert_eq!(all.pnl[0].value, 51.0);
        assert_eq!(all.pnl.last().unwrap().value, 51.0 + 5.0 * 40.0);
        assert_eq!(all.volume, Some(410.0));
    }

    #[test]
    fn an_address_is_its_accounts_together_ending_at_one_time() {
        // Two accounts asked for a moment apart: the second began later.
        let now = at(100.0) + 30.0;
        let main = History {
            hourly: climbing(&[at(98.0), at(99.0), at(100.0), now], 2.0),
            daily: climbing(&[at(100.0) - DAY, at(100.0), now], 5.0),
        };
        let sub = History {
            hourly: climbing(&[at(99.0), at(100.0), now + 2.0], 3.0),
            daily: climbing(&[at(100.0), now + 3.0], 7.0),
        };
        let performance = performance(vec![main, sub], now + 3.0);
        let week = performance.window(Period::Week, Scope::Perps).unwrap();
        let times: Vec<f64> = week
            .value
            .iter()
            .map(|point| point.time_ms / 1000.0)
            .collect();
        assert_eq!(times, [at(98.0), at(99.0), at(100.0), now + 2.0]);
        // Before it began, the second counts as what it first was.
        assert_eq!(week.value[0].value, 1_050.0 + 1_050.0);
        assert_eq!(week.value[3].value, 1_056.0 + 1_056.0);
        assert_eq!(week.pnl[0].value, 0.0);
        assert_eq!(week.pnl[3].value, 6.0 + 6.0);
        // Over all time it is worth nothing until it starts.
        let all = performance.window(Period::AllTime, Scope::Perps).unwrap();
        let worth: Vec<f64> = all.value.iter().map(|point| point.value).collect();
        assert_eq!(worth, [1_050.0, 1_055.0 + 1_050.0, 1_060.0 + 1_057.0]);
        let total = performance.window(Period::AllTime, Scope::Total).unwrap();
        assert_eq!(total.volume, Some(30.0 + 20.0));

        // An address with no account: every window, with nothing in it.
        let none = super::performance(Vec::new(), now);
        assert_eq!(none.0.len(), 8);
        let day = none.window(Period::Day, Scope::Total).unwrap();
        assert!(day.value.is_empty() && day.pnl.is_empty());
        assert_eq!(day.volume, Some(0.0));
    }
}
