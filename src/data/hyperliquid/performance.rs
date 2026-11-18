//! What a user's account was worth and made over time
//! ([`Info::Portfolio`](super::Info)), as Hyperliquid writes it and as the
//! portfolio reads it ([`Performance`]).
//!
//! The reply is a list of named windows: `day`, `week`, `month` and
//! `allTime` for the whole account, and the same four for its perpetuals
//! alone (`perpDay` ...). Each has the account's value and its profit and
//! loss as `[time in ms, figure]` samples, and what it traded (`vlm`). A
//! window under a name not known here is left out.

use serde::Deserialize;

use crate::data::{
    net::Figure,
    portfolio::{Performance, Period, Point, Scope, Window},
};

/// The reply to [`Info::Portfolio`](super::Info).
#[derive(Debug, Deserialize)]
pub(super) struct PortfolioReply(Vec<(String, WindowState)>);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WindowState {
    #[serde(default)]
    account_value_history: Vec<(f64, Figure)>,
    #[serde(default)]
    pnl_history: Vec<(f64, Figure)>,
    vlm: Option<Figure>,
}

/// The period and part of the account a window's name says.
fn named(name: &str) -> Option<(Period, Scope)> {
    Some(match name {
        "day" => (Period::Day, Scope::Total),
        "week" => (Period::Week, Scope::Total),
        "month" => (Period::Month, Scope::Total),
        "allTime" => (Period::AllTime, Scope::Total),
        "perpDay" => (Period::Day, Scope::Perps),
        "perpWeek" => (Period::Week, Scope::Perps),
        "perpMonth" => (Period::Month, Scope::Perps),
        "perpAllTime" => (Period::AllTime, Scope::Perps),
        _ => return None,
    })
}

/// A history's samples as points, oldest first.
fn points(samples: Vec<(f64, Figure)>) -> Vec<Point> {
    let mut points: Vec<Point> = samples
        .into_iter()
        .filter(|(time, _)| time.is_finite())
        .map(|(time_ms, Figure(value))| Point { time_ms, value })
        .collect();
    points.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
    points
}

impl From<PortfolioReply> for Performance {
    fn from(PortfolioReply(windows): PortfolioReply) -> Self {
        Self(
            windows
                .into_iter()
                .filter_map(|(name, state)| {
                    let (period, scope) = named(&name)?;
                    Some(Window {
                        period,
                        scope,
                        value: points(state.account_value_history),
                        pnl: points(state.pnl_history),
                        volume: state.vlm.map(|Figure(volume)| volume),
                    })
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_reply_reads_as_the_windows_it_names() {
        let window = |value: f64| {
            json!({
                "accountValueHistory": [
                    [1_741_886_630_493_u64, "100.0"],
                    [1_741_895_270_493_u64, format!("{value}")],
                ],
                "pnlHistory": [
                    [1_741_895_270_493_u64, "-4.5"],
                    [1_741_886_630_493_u64, "0.0"],
                ],
                "vlm": "2500.75",
            })
        };
        let reply: PortfolioReply = serde_json::from_value(json!([
            ["day", window(95.5)],
            ["week", window(120.0)],
            ["month", { "accountValueHistory": [], "pnlHistory": [], "vlm": "0.0" }],
            ["allTime", window(300.0)],
            ["perpDay", window(80.0)],
            ["perpWeek", window(81.0)],
            ["perpMonth", window(82.0)],
            ["perpAllTime", window(83.0)],
            ["quarter", window(1.0)],
        ]))
        .expect("the documented reply reads");
        let performance = Performance::from(reply);
        assert_eq!(performance.0.len(), 8, "the unknown window is left out");

        let day = performance.window(Period::Day, Scope::Total).unwrap();
        assert_eq!(
            day.value,
            [
                Point {
                    time_ms: 1_741_886_630_493.0,
                    value: 100.0
                },
                Point {
                    time_ms: 1_741_895_270_493.0,
                    value: 95.5
                },
            ]
        );
        // Samples are kept oldest first, however they were written.
        assert_eq!(day.pnl[0].value, 0.0);
        assert_eq!(day.pnl[1].value, -4.5);
        assert_eq!(day.volume, Some(2500.75));

        let perps = performance.window(Period::AllTime, Scope::Perps).unwrap();
        assert_eq!(perps.value[1].value, 83.0);
        let month = performance.window(Period::Month, Scope::Total).unwrap();
        assert!(month.value.is_empty() && month.pnl.is_empty());
        assert_eq!(month.volume, Some(0.0));
    }
}
