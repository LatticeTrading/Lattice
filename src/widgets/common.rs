//! Building blocks several widgets use: the market picker, exchange logos,
//! market tags, a settings panel's market box, the compact number field and
//! double clicks.
//! [`Shared`] (signals of big values) and [`Keyed`] (watches and leases kept
//! in step with what is wanted) are the data layer's, named here too for
//! the widgets that use them. What every widget needs (app-wide controls,
//! panels, list scrolling) is in `crate::ui`.

mod clicks;
mod exchange_logo;
mod market_chooser;
mod market_picker;
mod market_tags;
mod number_field;

pub(crate) use self::{
    clicks::{CLICK_SLOP, Clicks},
    exchange_logo::{LogoStyle, bound_exchange_logo, exchange_logo},
    market_chooser::{market_chooser, market_name},
    market_picker::MarketPicker,
    market_tags::{market_tags, tag_text},
    number_field::number_field,
};
pub(crate) use crate::data::{Keyed, Shared};
