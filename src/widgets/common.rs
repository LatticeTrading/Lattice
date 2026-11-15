//! Building blocks several widgets use: the market picker, exchange logos,
//! market tags, a settings panel's market box, the compact number field,
//! double clicks, wheel steps, a canvas pen, and what the widgets that
//! draw order books share ([`books`]).
//! [`Shared`] (signals of big values) and [`Keyed`] (watches and leases kept
//! in step with what is wanted) are the data layer's, named here too for
//! the widgets that use them. What every widget needs (app-wide controls,
//! panels, list scrolling) is in `crate::ui`.

pub(crate) mod books;
mod clicks;
mod exchange_logo;
mod market_chooser;
mod market_picker;
mod market_tags;
mod number_field;
mod pen;
mod wheel;

pub(crate) use self::{
    clicks::{CLICK_SLOP, Clicks},
    exchange_logo::{LogoStyle, bound_exchange_logo, exchange_logo},
    market_chooser::{market_chooser, market_name},
    market_picker::MarketPicker,
    market_tags::{market_tags, tag_text},
    number_field::number_field,
    pen::{FONT, Pen, SMALL_FONT, width_of},
    wheel::{wheel_factor, wheel_steps},
};
pub(crate) use crate::data::{Keyed, Shared};
