//! Building blocks several widgets use: the market picker, exchange logos,
//! market tags, the compact number field, [`Shared`] for signals of big
//! values, and [`Keyed`] for keeping watches and leases in step with what
//! is wanted. What every widget needs (app-wide controls, panels, list
//! scrolling) is in `crate::ui`.

mod exchange_logo;
mod keyed;
mod market_picker;
mod market_tags;
mod number_field;
mod shared;

pub(crate) use self::{
    exchange_logo::{LogoStyle, bound_exchange_logo, exchange_logo},
    keyed::Keyed,
    market_picker::MarketPicker,
    market_tags::{market_tags, tag_text},
    number_field::number_field,
    shared::Shared,
};
