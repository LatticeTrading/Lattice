//! Building blocks several widgets use: the market picker, exchange logos,
//! market tags, a settings panel's market box, the compact number field,
//! double clicks, wheel steps, a canvas pen, the credit of an outside
//! source of data ([`source_credit`]), what the widgets that draw
//! order books share ([`books`]), what the ones that plot a series over
//! time do ([`plot`]), and what the ones that show an account do
//! ([`accounts`]).
//! [`Shared`] (signals of big values) and [`Keyed`] (watches and leases kept
//! in step with what is wanted) are the data layer's, named here too for
//! the widgets that use them. What every widget needs (app-wide controls,
//! panels, list scrolling) is in `crate::ui`.

pub(crate) mod accounts;
pub(crate) mod books;
mod clicks;
#[cfg(test)]
pub(crate) mod defillama;
mod exchange_logo;
mod market_chooser;
mod market_picker;
mod market_tags;
mod number_field;
mod pen;
pub(crate) mod plot;
mod source_credit;
mod wheel;

pub(crate) use self::{
    clicks::{CLICK_SLOP, Clicks},
    exchange_logo::{LogoStyle, bound_exchange_logo, exchange_logo},
    market_chooser::{empty_market_chooser, market_chooser, market_name},
    market_picker::MarketPicker,
    market_tags::{market_tags, tag_segment, tag_text},
    number_field::number_field,
    pen::{FONT, Pen, SMALL_FONT, width_of},
    source_credit::source_credit,
    wheel::{wheel_factor, wheel_steps},
};
pub(crate) use crate::data::{Keyed, Shared};
