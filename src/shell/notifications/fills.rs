//! What the shell posts of the connected wallet's account: its trades, as
//! each exchange it can be traded on says them.
//!
//! The account is followed on every such exchange from the moment the
//! wallet connects ([`Trading::follow`]), whether a widget shows it or
//! not, so an order that fills while the user is on another page is told
//! all the same. The stream is the one the widgets that follow the account
//! share, and the connection panel names this hold `Notifications`. An
//! exchange added to trading is followed with no change here; one with no
//! stream of an account has nothing to tell.

use nsh_core::prelude::*;

use crate::{
    data::{
        Keyed, Trading,
        net::Consumer,
        portfolio::{Account, Fill},
        trading::{Access, AccountEvent, Following},
    },
    format,
    notify::{Level, Notice, Notifications},
    widgets::common::orders::fill_line,
};

/// Who holds the streams this follows, as the connection panel names it.
const HOLDER: Consumer = Consumer::Part("Notifications");

/// Posts to `center` what happens to the connected wallet's account on
/// each of `trading`'s exchanges, for as long as the current owner (the
/// shell) lives.
pub(in crate::shell) fn announce(trading: Trading, center: Notifications) {
    let following = Keyed::<(Account, bool), Option<Following>>::default();
    effect(move || {
        // Asked for again when trading comes to be enabled for the account
        // or stops being: an exchange may stream an account only to what
        // the account allowed.
        let wanted: Vec<_> = trading
            .venues()
            .filter_map(|venue| {
                let account = trading.account(venue.venue())?;
                let enabled = trading.access(venue.venue()) == Access::Ready;
                Some(((account.clone(), enabled), (account, venue.name())))
            })
            .collect();
        untracked(|| {
            following.follow(
                wanted,
                |_, _| true,
                |(account, venue)| {
                    let venue = *venue;
                    let hears = move |event: &AccountEvent| match event {
                        AccountEvent::Filled(fill) => center.post(filled(fill, venue)),
                    };
                    HOLDER.scope(|| trading.follow(account, hears)).ok()
                },
            );
        });
    });
}

/// A trade of the account's on `venue`, as a notification: what it did in
/// the exchange's words, what traded as the trading widgets say it, and
/// what closing made or lost.
fn filled(fill: &Fill, venue: &str) -> Notice {
    let title = match &fill.action {
        Some(action) => format!("Filled: {action}"),
        None => "Order filled".to_owned(),
    };
    let mut body = fill_line(fill);
    if let Some(made) = fill.closed_pnl {
        body.push_str(&format!(" Closed PnL {}.", format::signed_usd(made)));
    }
    Notice::new(Level::Success, title).body(body).origin(venue)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::portfolio::TradeSide;

    #[test]
    fn a_fill_says_what_it_did_what_traded_and_what_closing_made() {
        let opened = Fill {
            id: "1".to_owned(),
            time_ms: 0.0,
            instrument: "BTC".to_owned(),
            side: TradeSide::Buy,
            action: Some("Open Long".to_owned()),
            price: 61_000.0,
            size: 0.01,
            fee: None,
            fee_asset: None,
            closed_pnl: None,
            taker: None,
        };
        assert_eq!(
            filled(&opened, "Hyperliquid"),
            Notice {
                level: Level::Success,
                title: "Filled: Open Long".to_owned(),
                body: Some("Bought 0.01 BTC at 61000.0.".to_owned()),
                origin: Some("Hyperliquid".to_owned()),
            }
        );
        // An exchange that does not say what a trade did, and one that
        // closed at a loss.
        let closed = Fill {
            side: TradeSide::Sell,
            action: None,
            closed_pnl: Some(-4.1),
            ..opened
        };
        let notice = filled(&closed, "Paper");
        assert_eq!(notice.title, "Order filled");
        assert_eq!(
            notice.body.as_deref(),
            Some("Sold 0.01 BTC at 61000.0. Closed PnL -$4.10.")
        );
    }
}
