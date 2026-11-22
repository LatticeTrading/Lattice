//! How an order is typed and said, the same wherever one is sent from (the
//! Trade Panel's ticket, the cell that closes a position in the Positions):
//!
//! - [`typed`]: what was typed as a number;
//! - [`order_line`]: an order in words, as the cell that sends it and what
//!   is said of it name it;
//! - [`placed_line`]: what became of one the exchange took;
//! - [`fill_line`]: a trade of the account's, as it is said when it
//!   happens.

use crate::{
    data::{
        portfolio::{Fill, TradeSide},
        trading::{Exits, OrderKind, OrderRequest, Placed, Side},
    },
    format,
};

/// What was typed as a number. Only a plain decimal is one: `1,5` is not
/// read as fifteen, nor as one and a half.
pub(crate) fn typed(text: &str) -> Option<f64> {
    let text = text.trim();
    let plain = !text.is_empty()
        && text
            .bytes()
            .all(|digit| digit.is_ascii_digit() || digit == b'.');
    plain
        .then(|| text.parse::<f64>().ok())
        .flatten()
        .filter(|number| number.is_finite())
}

/// `, TP 65000, SL 58000`: `exits` as they follow what they are placed
/// with; nothing for none. Figures as they were typed.
fn exits_line(exits: Exits) -> String {
    let mut line = String::new();
    if let Some(price) = exits.take_profit {
        line.push_str(&format!(", TP {price}"));
    }
    if let Some(price) = exits.stop_loss {
        line.push_str(&format!(", SL {price}"));
    }
    line
}

/// `Buy 0.01 BTC at 61000`, `Sell 2 ETH at market`, with its exits after
/// it (`, TP 65000, SL 58000`): what `order` is, as its button and its
/// results say it. Figures as they were typed.
pub(crate) fn order_line(order: &OrderRequest) -> String {
    let at = match order.kind {
        OrderKind::Limit { price, .. } => format!("{price}"),
        OrderKind::Market { .. } => "market".to_owned(),
    };
    format!(
        "{} {} {} at {at}{}",
        order.side.label(),
        order.size,
        order.instrument,
        exits_line(order.exits)
    )
}

/// `Bought 0.01 BTC at 60990.0.`: something that traded, as it is said
/// whether the exchange answered an order with it or its stream told of it.
fn traded_line(bought: bool, size: f64, instrument: &str, price: f64) -> String {
    format!(
        "{} {} {instrument} at {}.",
        if bought { "Bought" } else { "Sold" },
        format::quantity(size),
        format::price_text(price)
    )
}

/// What became of `order`, in words.
pub(crate) fn placed_line(order: &OrderRequest, placed: &Placed) -> String {
    match placed {
        Placed::Resting { id } => format!("Resting: {} (order {id}).", order_line(order)),
        Placed::Filled { size, price, .. } => {
            traded_line(order.side == Side::Buy, *size, &order.instrument, *price)
        }
        Placed::Accepted => format!("Accepted: {}.", order_line(order)),
    }
}

/// A fill of the account's, as the panel says it when it happens.
pub(crate) fn fill_line(fill: &Fill) -> String {
    traded_line(
        fill.side == TradeSide::Buy,
        fill.size,
        &fill.instrument,
        fill.price,
    )
}
