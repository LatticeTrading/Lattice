//! What a user made and traded over a period ([`Info::UserOverview`](super::Info)),
//! as TxFlow writes it and as the portfolio reads it ([`Performance`]).
//!
//! TxFlow reports a period's figures, not samples of it: what the account
//! made over the period (`pnl`), what it traded (`volume`), its return and
//! drawdown in percent and its equity now (`totalEquity`). So a window is
//! what can be said of it without a guess:
//!
//! - what it made is two points, nothing at the period's start (where the
//!   count begins) and `pnl` at the reply's time, so what the window made
//!   (the last less the first) is TxFlow's figure. Between them it is not
//!   known, and a chart of it is a line from one to the other;
//! - what it was worth is not reported: the equity now says nothing of the
//!   period's start, which is what a window's worth is measured from;
//! - what it traded is `volume`.
//!
//! A window is one request: the day, the week and the month
//! ([`OverviewPeriod::ALL`]), each of the perpetuals sub-account alone
//! ([`Scope::Perps`]) and of it with the Outcome one ([`Scope::Total`]).
//! All time is not asked for: TxFlow reports it, but not from when, so its
//! first point has no time.

use serde::Deserialize;

use super::OverviewPeriod;
use crate::data::{
    net::Figure,
    portfolio::{Period, Point, Scope, Window},
};

/// The reply to [`Info::UserOverview`](super::Info). `pnl` is required, so
/// that a reply that is a refusal does not read as an account that made
/// nothing.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Overview {
    pnl: Figure,
    volume: Option<Figure>,
}

impl OverviewPeriod {
    /// The portfolio's period it is.
    pub(super) const fn period(self) -> Period {
        match self {
            Self::Day => Period::Day,
            Self::Week => Period::Week,
            Self::Month => Period::Month,
        }
    }
}

impl Overview {
    /// The window of `period` and `scope` it is, as it stands at `now_ms`.
    pub(super) fn window(self, period: OverviewPeriod, scope: Scope, now_ms: f64) -> Window {
        Window {
            period: period.period(),
            scope,
            value: Vec::new(),
            pnl: vec![
                Point {
                    time_ms: now_ms - period.span_ms(),
                    value: 0.0,
                },
                Point {
                    time_ms: now_ms,
                    value: self.pnl.0,
                },
            ],
            volume: self.volume.map(|Figure(volume)| volume),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::data::portfolio::Performance;

    const NOW: f64 = 1_795_087_837_004.0;

    #[test]
    fn a_period_reads_as_what_it_made_from_nothing_at_its_start() {
        // As TxFlow wrote it (checked live 2026-11-19).
        let reply: Overview = serde_json::from_value(json!({
            "code": 0, "msg": "", "period": "7D", "pnl": "542668.66", "roe": "10.68",
            "volume": "564426724.62", "maxDrawdown": "4.52", "totalEquity": "5622102.60"
        }))
        .expect("TxFlow's reply reads");
        let window = reply.window(OverviewPeriod::Week, Scope::Total, NOW);
        assert_eq!((window.period, window.scope), (Period::Week, Scope::Total));
        assert!(window.value.is_empty(), "its worth is not reported");
        assert_eq!(window.volume, Some(564_426_724.62));
        let [start, end] = window.pnl.as_slice() else {
            panic!("two points: {:?}", window.pnl);
        };
        assert_eq!(
            (start.time_ms, start.value),
            (NOW - 7.0 * 86_400_000.0, 0.0)
        );
        assert_eq!((end.time_ms, end.value), (NOW, 542_668.66));

        // The portfolio finds it.
        let history = Performance(vec![window]);
        assert!(history.window(Period::Week, Scope::Total).is_some());
        assert!(history.window(Period::AllTime, Scope::Total).is_none());
    }

    #[test]
    fn a_refusal_is_no_period() {
        let refused = json!({ "success": false, "code": 10000, "message": "invalid user address" });
        assert!(serde_json::from_value::<Overview>(refused).is_err());
        let idle: Overview = serde_json::from_value(json!({
            "code": 0, "msg": "", "period": "24H", "pnl": "0", "roe": "0", "volume": "0"
        }))
        .unwrap();
        let window = idle.window(OverviewPeriod::Day, Scope::Perps, NOW);
        assert_eq!(window.pnl[1].value, 0.0);
        assert_eq!(window.volume, Some(0.0));
    }
}
