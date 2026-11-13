//! The exchanges Lattice shows: display names and logos, by Ferris's
//! exchange id (`bybit`).
//!
//! Logos are the old app's artwork (`exchangeLogoAssets.ts`), one SVG per
//! exchange in `assets/exchanges/`, inlined at compile time so desktop and
//! web draw the same thing. Marks drawn in `currentColor` follow the text
//! color they are drawn in; the others keep their brand colors.

use std::{cell::RefCell, collections::HashMap};

use nsh_core::element::ImageSrc;

/// An exchange's name for people, or its id when Lattice does not know it.
pub fn display_name(exchange: &str) -> &str {
    match exchange {
        "apex" => "Apex",
        "aster" => "Aster",
        "binance" => "Binance",
        "bitfinex" => "Bitfinex",
        "bybit" => "Bybit",
        "extended" => "Extended",
        "hyperliquid" => "Hyperliquid",
        "kucoin" => "KuCoin",
        "lighterxyz" => "Lighter",
        "nado" => "Nado",
        "perpl" => "Perpl",
        "phoenix" => "Phoenix",
        "variational" => "Variational",
        other => other,
    }
}

/// The logo's markup, by the id it is filed under.
fn markup(id: &str) -> Option<&'static str> {
    macro_rules! logos {
        ($($id:literal),* $(,)?) => {
            match id {
                $($id => Some(include_str!(concat!("../assets/exchanges/", $id, ".svg"))),)*
                _ => None,
            }
        };
    }
    logos!(
        "apex",
        "aster",
        "binance",
        "bitfinex",
        "bybit",
        "extended",
        "hyperliquid",
        "kucoin",
        "lighterxyz",
        "nado",
        "perpl",
        "phoenix",
        "variational",
    )
}

/// The id an exchange's logo is filed under: venue families share one
/// (`binanceusdm`, `hyperliquid-testnet`) and some have short names.
fn logo_id(exchange: &str) -> String {
    let id = exchange.trim().to_ascii_lowercase();
    for family in ["hyperliquid", "lighterxyz", "binance"] {
        if id.contains(family) {
            return family.to_owned();
        }
    }
    match id.as_str() {
        "hl" => "hyperliquid".to_owned(),
        "kucoinfutures" => "kucoin".to_owned(),
        _ => id,
    }
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
            .or_insert_with(|| markup(&logo_id(exchange)).map(ImageSrc::svg_markup))
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
