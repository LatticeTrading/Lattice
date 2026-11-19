//! What a user pays to trade ([`Info::UserFees`](super::Info)), as TxFlow
//! writes it and as the portfolio reads it ([`Fees`]).
//!
//! - The rates are the user's own for perpetuals (`userCrossRate` taking,
//!   `userAddRate` adding liquidity), every discount already taken off.
//!   They are asked of BTC's perpetual ([`FEES_ASSET`]): TxFlow asks for an
//!   instrument, and every perpetual has the one schedule. Spot's are not
//!   reported.
//! - The tier is counted on the last [`DAYS`] days' volume, today's not
//!   among them. The reply has the user's volume day by day
//!   (`dailyUserVlm`, each dated by its start in Unix milliseconds); each
//!   of its figures is the whole day's volume (checked live 2026-11-19:
//!   `userCross`, `userAdd` and `exchange` are the same), so one is
//!   counted. The tiers are the fee schedule's (`tiers.vip`), each from a
//!   volume (`ntlCutoff`), the first from nothing: that one is the base
//!   rate, tier 0.
//! - The discount is the referral discount (`activeReferralDiscount`).

use serde::Deserialize;

use crate::data::{net::Figure, portfolio::Fees};

/// How many days' volume the tier is counted on.
pub(super) const DAYS: usize = 14;
/// The instrument the rates are asked of: BTC's perpetual.
pub(super) const FEES_ASSET: &str = "1";
/// A day, in milliseconds.
const DAY_MS: f64 = 86_400_000.0;

/// The reply to [`Info::UserFees`](super::Info).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UserFees {
    #[serde(default)]
    daily_user_vlm: Vec<DailyVolume>,
    fee_schedule: Option<FeeSchedule>,
    user_cross_rate: Option<Figure>,
    user_add_rate: Option<Figure>,
    active_referral_discount: Option<Figure>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DailyVolume {
    /// The day's start in Unix milliseconds, as text.
    date: Figure,
    user_cross: Option<Figure>,
}

#[derive(Debug, Deserialize)]
struct FeeSchedule {
    tiers: Option<Tiers>,
}

#[derive(Debug, Deserialize)]
struct Tiers {
    #[serde(default)]
    vip: Vec<Tier>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Tier {
    ntl_cutoff: Figure,
}

/// A figure's number, when there is one.
fn number(figure: Option<Figure>) -> Option<f64> {
    figure.map(|Figure(number)| number)
}

impl UserFees {
    /// What the user pays, as it stands at `now_ms`.
    pub(super) fn fees(self, now_ms: f64) -> Fees {
        let today = (now_ms / DAY_MS).floor() * DAY_MS;
        let mut days: Vec<DailyVolume> = self
            .daily_user_vlm
            .into_iter()
            .filter(|day| day.date.0 < today)
            .collect();
        days.sort_by(|a, b| a.date.0.total_cmp(&b.date.0));
        let counted = &days[days.len().saturating_sub(DAYS)..];
        let volume = (!counted.is_empty()).then(|| {
            counted
                .iter()
                .map(|day| number(day.user_cross).unwrap_or(0.0))
                .sum::<f64>()
        });
        // The tiers above the base, each from the volume it starts at.
        let mut cutoffs: Vec<f64> = self
            .fee_schedule
            .and_then(|schedule| schedule.tiers)
            .map(|tiers| tiers.vip.iter().map(|tier| tier.ntl_cutoff.0).collect())
            .unwrap_or_default();
        cutoffs.retain(|cutoff| *cutoff > 0.0);
        cutoffs.sort_by(f64::total_cmp);
        let tier = volume.map(|volume| cutoffs.iter().filter(|cutoff| **cutoff <= volume).count());
        Fees {
            taker: number(self.user_cross_rate),
            maker: number(self.user_add_rate),
            spot_taker: None,
            spot_maker: None,
            volume,
            volume_days: DAYS as u32,
            tier,
            tier_floor: tier.map(|tier| tier.checked_sub(1).map_or(0.0, |below| cutoffs[below])),
            next_tier: tier.and_then(|tier| cutoffs.get(tier).copied()),
            discount: number(self.active_referral_discount),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// The day after the last one in [`reply`], at 11:34 UTC.
    const NOW: f64 = 1_795_088_069_361.0;

    /// As TxFlow wrote it (checked live 2026-11-19), cut to three days and
    /// with today's added.
    fn reply() -> UserFees {
        serde_json::from_value(json!({
            "activeReferralDiscount": "0.000000",
            "dailyUserVlm": [
                { "date": "1794787200000", "exchange": "300000000.00",
                  "userAdd": "300000000.00", "userCross": "300000000.00" },
                { "date": "1794960000000", "exchange": "84073085.38",
                  "userAdd": "84073085.38", "userCross": "84073085.38" },
                { "date": "1794873600000", "exchange": "200000000.00",
                  "userAdd": "200000000.00", "userCross": "200000000.00" },
                { "date": "1795046400000", "exchange": "9.0", "userAdd": "9.0", "userCross": "9.0" }
            ],
            "feeSchedule": {
                "add": "0.000150", "cross": "0.000450", "referralDiscount": "0.000000",
                "tiers": { "mm": [], "vip": [
                    { "add": "0.000150", "cross": "0.000450", "ntlCutoff": "0.00" },
                    { "add": "0.000120", "cross": "0.000400", "ntlCutoff": "5000000.00" },
                    { "add": "0.000080", "cross": "0.000350", "ntlCutoff": "25000000.00" },
                    { "add": "0.000040", "cross": "0.000300", "ntlCutoff": "100000000.00" },
                    { "add": "0.000000", "cross": "0.000280", "ntlCutoff": "500000000.00" },
                    { "add": "0.000000", "cross": "0.000260", "ntlCutoff": "1000000000.00" }
                ] }
            },
            "userAdd": "0.000000", "userCross": "0.000280",
            "userAddRate": "0.000000", "userCrossRate": "0.000280",
            "volumeToday": "26611022.19"
        }))
        .expect("TxFlow's reply reads")
    }

    #[test]
    fn fees_read_as_the_rates_and_the_tier_of_the_days_before_today() {
        let fees = reply().fees(NOW);
        assert_eq!((fees.taker, fees.maker), (Some(0.000_28), Some(0.0)));
        assert_eq!((fees.spot_taker, fees.spot_maker), (None, None));
        // Today's is left out; each day once.
        assert!((fees.volume.unwrap() - 584_073_085.38).abs() < 1e-3);
        assert_eq!(fees.volume_days, 14);
        assert_eq!(fees.tier, Some(4));
        assert_eq!(fees.tier_floor, Some(500_000_000.0));
        assert_eq!(fees.next_tier, Some(1_000_000_000.0));
        assert_eq!(fees.discount, Some(0.0));
    }

    #[test]
    fn a_user_that_never_traded_is_on_the_base_rate() {
        let fees: UserFees = serde_json::from_value(json!({
            "dailyUserVlm": [],
            "feeSchedule": { "tiers": { "vip": [
                { "ntlCutoff": "0.00" }, { "ntlCutoff": "5000000.00" }
            ] } },
            "userCrossRate": "0.000450", "userAddRate": "0.000150"
        }))
        .unwrap();
        let fees = fees.fees(NOW);
        assert_eq!((fees.volume, fees.tier), (None, None));
        assert_eq!(fees.taker, Some(0.000_45));
    }
}
