//! What a user pays to trade ([`Info::UserFees`](super::Info)), as
//! Hyperliquid writes it and as the portfolio reads it ([`Fees`]).
//!
//! - The rates are the user's own (`userCrossRate`, `userAddRate` and their
//!   spot pair), every discount already taken off.
//! - The tier is counted on the last [`DAYS`] days' volume: the reply has
//!   the user's volume day by day (`dailyUserVlm`, taking and adding
//!   liquidity apart), the newest being today's, which is still running and
//!   is not counted. The tiers are the fee schedule's (`tiers.vip`), each
//!   from a volume (`ntlCutoff`); below the first is the base rate, tier 0.
//! - The discount is the staking discount and the referral discount
//!   together, one taken off what the other leaves.

use serde::Deserialize;

use crate::data::{net::Figure, portfolio::Fees};

/// How many days' volume the tier is counted on.
const DAYS: usize = 14;

/// The reply to [`Info::UserFees`](super::Info).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UserFees {
    #[serde(default)]
    daily_user_vlm: Vec<DailyVolume>,
    fee_schedule: Option<FeeSchedule>,
    user_cross_rate: Option<Figure>,
    user_add_rate: Option<Figure>,
    user_spot_cross_rate: Option<Figure>,
    user_spot_add_rate: Option<Figure>,
    active_referral_discount: Option<Figure>,
    active_staking_discount: Option<StakingDiscount>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DailyVolume {
    /// `2026-11-03`, which sorts as the days do.
    date: String,
    user_cross: Option<Figure>,
    user_add: Option<Figure>,
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

#[derive(Debug, Deserialize)]
struct StakingDiscount {
    discount: Option<Figure>,
}

/// A figure's number, when there is one.
fn number(figure: Option<Figure>) -> Option<f64> {
    figure.map(|Figure(number)| number)
}

impl From<UserFees> for Fees {
    fn from(fees: UserFees) -> Self {
        let mut days = fees.daily_user_vlm;
        days.sort_by(|a, b| a.date.cmp(&b.date));
        // Today's is still running.
        days.pop();
        let counted = &days[days.len().saturating_sub(DAYS)..];
        let volume = (!counted.is_empty()).then(|| {
            counted
                .iter()
                .map(|day| {
                    number(day.user_cross).unwrap_or(0.0) + number(day.user_add).unwrap_or(0.0)
                })
                .sum::<f64>()
        });
        let mut cutoffs: Vec<f64> = fees
            .fee_schedule
            .and_then(|schedule| schedule.tiers)
            .map(|tiers| tiers.vip.iter().map(|tier| tier.ntl_cutoff.0).collect())
            .unwrap_or_default();
        cutoffs.sort_by(f64::total_cmp);
        let tier = volume.map(|volume| cutoffs.iter().filter(|cutoff| **cutoff <= volume).count());
        let staking = fees
            .active_staking_discount
            .and_then(|staking| number(staking.discount));
        let referral = number(fees.active_referral_discount);
        let discount = match (staking, referral) {
            (None, None) => None,
            (staking, referral) => {
                Some(1.0 - (1.0 - staking.unwrap_or(0.0)) * (1.0 - referral.unwrap_or(0.0)))
            }
        };
        Self {
            taker: number(fees.user_cross_rate),
            maker: number(fees.user_add_rate),
            spot_taker: number(fees.user_spot_cross_rate),
            spot_maker: number(fees.user_spot_add_rate),
            volume,
            volume_days: DAYS as u32,
            tier,
            tier_floor: tier.map(|tier| tier.checked_sub(1).map_or(0.0, |below| cutoffs[below])),
            next_tier: tier.and_then(|tier| cutoffs.get(tier).copied()),
            discount,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    /// A reply whose days each traded `per_day`, today's `today`.
    fn reply(per_day: f64, today: f64) -> Value {
        let mut days: Vec<Value> = (1..=15)
            .map(|day| {
                json!({
                    "date": format!("2026-11-{day:02}"),
                    "userCross": format!("{}", per_day * 0.75),
                    "userAdd": format!("{}", per_day * 0.25),
                    "exchange": "5488584292.45"
                })
            })
            .collect();
        days.push(
            json!({ "date": "2026-11-16", "userCross": format!("{today}"), "userAdd": "0.0" }),
        );
        // Written in no particular order.
        days.reverse();
        json!({
            "dailyUserVlm": days,
            "feeSchedule": {
                "cross": "0.00045", "add": "0.00015",
                "tiers": { "vip": [
                    { "ntlCutoff": "25000000.0", "cross": "0.00035" },
                    { "ntlCutoff": "5000000.0", "cross": "0.0004" },
                    { "ntlCutoff": "100000000.0", "cross": "0.0003" }
                ], "mm": [] }
            },
            "userCrossRate": "0.00036",
            "userAddRate": "0.000108",
            "userSpotCrossRate": "0.00054",
            "userSpotAddRate": "0.00027",
            "activeReferralDiscount": "0.0",
            "activeStakingDiscount": { "bpsOfMaxSupply": "0.001", "discount": "0.1" }
        })
    }

    #[test]
    fn fees_read_as_the_rates_and_the_tier_the_last_two_weeks_reach() {
        let fees = Fees::from(serde_json::from_value::<UserFees>(reply(500_000.0, 9e9)).unwrap());
        assert_eq!(fees.taker, Some(0.000_36));
        assert_eq!(fees.maker, Some(0.000_108));
        assert_eq!(
            (fees.spot_taker, fees.spot_maker),
            (Some(0.000_54), Some(0.000_27))
        );
        // Fourteen whole days, not today's.
        assert_eq!(fees.volume, Some(7_000_000.0));
        assert_eq!(fees.volume_days, 14);
        assert_eq!(fees.tier, Some(1));
        assert_eq!(fees.tier_floor, Some(5_000_000.0));
        assert_eq!(fees.next_tier, Some(25_000_000.0));
        assert!((fees.discount.unwrap() - 0.1).abs() < 1e-12);

        // Below the first cutoff is the base; above the last, no next.
        let base = Fees::from(serde_json::from_value::<UserFees>(reply(1_000.0, 0.0)).unwrap());
        assert_eq!((base.tier, base.tier_floor), (Some(0), Some(0.0)));
        assert_eq!(base.next_tier, Some(5_000_000.0));
        let top = Fees::from(serde_json::from_value::<UserFees>(reply(1e8, 0.0)).unwrap());
        assert_eq!((top.tier, top.next_tier), (Some(3), None));

        // An account that never traded.
        let none = Fees::from(serde_json::from_value::<UserFees>(json!({})).unwrap());
        assert_eq!((none.volume, none.tier, none.discount), (None, None, None));
    }
}
