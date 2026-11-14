//! The exchanges Lattice shows: display names, order and logos, by Ferris's
//! exchange id (`bybit`), read from the venue table
//! ([`VENUES`](crate::data::ferris::VENUES)).
//!
//! Logos are inlined at compile time so desktop and web draw the same
//! thing.

use std::{cell::RefCell, collections::HashMap};

use nsh_core::element::ImageSrc;

use crate::data::ferris::{VENUES, Venue};

/// An exchange's name for people, or its id when Lattice does not know it.
pub fn display_name(exchange: &str) -> &str {
    Venue::listed(exchange).map_or(exchange, |venue| venue.name)
}

/// Where `exchange` comes in lists of exchanges: the old app's order
/// (`SUPPORTED_EXCHANGES`, the table's), then any other by id.
pub fn rank(exchange: &str) -> (usize, &str) {
    let at = VENUES
        .iter()
        .position(|venue| venue.id == exchange)
        .unwrap_or(VENUES.len());
    (at, exchange)
}

/// The logo's markup of `exchange` (any case): its venue's, a family's
/// whose id it contains (`binanceusdm`, `hyperliquid-testnet`), or the one
/// a short name is filed under (`hl`).
fn markup(exchange: &str) -> Option<&'static str> {
    let id = exchange.trim().to_ascii_lowercase();
    VENUES
        .iter()
        .find(|venue| venue.logo_family && id.contains(venue.id))
        .or_else(|| {
            VENUES
                .iter()
                .find(|venue| venue.id == id || venue.logo_aliases.contains(&id.as_str()))
        })
        .map(|venue| venue.logo)
}

thread_local! {
    /// Each logo's source, made once: a source is the markup encoded as a
    /// `data:` URI, which every cell showing the logo then shares.
    static LOGOS: RefCell<HashMap<String, Option<ImageSrc>>> = RefCell::new(HashMap::new());
}

/// The exchange's logo, `None` when Lattice has no artwork for it.
pub fn logo(exchange: &str) -> Option<ImageSrc> {
    LOGOS.with(|logos| {
        logos
            .borrow_mut()
            .entry(exchange.to_owned())
            .or_insert_with(|| markup(exchange).map(ImageSrc::svg_markup))
            .clone()
    })
}

/// A source that draws nothing: what a logo slot holds while its exchange
/// has no logo (its initials show instead).
pub fn blank_logo() -> ImageSrc {
    thread_local! {
        static BLANK: ImageSrc =
            ImageSrc::svg_markup(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"/>"#);
    }
    BLANK.with(Clone::clone)
}

/// Two letters for an exchange with no logo.
pub fn initials(exchange: &str) -> String {
    let initials: String = exchange.trim().chars().take(2).collect();
    if initials.is_empty() {
        "--".to_owned()
    } else {
        initials.to_uppercase()
    }
}
