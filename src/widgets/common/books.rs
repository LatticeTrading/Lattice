//! What the widgets that draw order books share (the Order Book and the
//! Depth Chart), so that it exists once:
//!
//! - [`settings`]: the two books a widget draws ([`Pair`], [`Layer`]),
//!   their depth and their price grouping ([`Books`]), saved under the old
//!   app's names, and how a widget's settings hold them
//!   ([`BookSettings`]);
//! - [`levels`]: one side of a book as it is shown, level by level or in
//!   price groups ([`pack`]);
//! - [`feeds`]: a feed lease per book, and drawing what they bring at most
//!   once a frame ([`BookFeeds`]);
//! - [`bar`], [`panel`]: the top bar (a segment per book, the Group and
//!   Depth selects, the notices of indicative books) and the settings
//!   panel's rows for the same, with the market picker both open;
//! - [`colors`]: the colors two books are told apart by, for a canvas
//!   ([`Colors`]) and for a shader ([`Palette`]);
//! - [`drawn`]: what a canvas overlay hands its view ([`Drawn`]: the
//!   drawing, and the exchange logos that go over it), and what a plot
//!   says while it has no book ([`status`]).

mod bar;
mod colors;
mod drawn;
mod feeds;
mod levels;
mod panel;
mod settings;
#[cfg(test)]
pub(crate) mod testing;

#[cfg(test)]
pub(crate) use self::drawn::{UNAVAILABLE, WAITING};
pub(crate) use self::{
    bar::{
        NARROW, book_name, depth_select, grouping_select, logo_and_pair, notices, picker,
        quick_select, shown_text, top_bar,
    },
    colors::{Colors, Palette},
    drawn::{Drawn, Logo, logos, status},
    feeds::BookFeeds,
    levels::{Side, pack, round},
    panel::{depth_row, grouping_row, market_row},
    settings::{
        BookLayout, BookSettings, Books, DEPTH_PRESETS, Grouping, Layer, MAX_DEPTH, MIN_DEPTH, Pair,
    },
};
