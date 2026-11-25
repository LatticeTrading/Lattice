//! The Watchlist: the screener's table over lists of markets the user
//! makes (perpetuals and futures, picked from the market picker), with the
//! same columns, figures, sorting and settings.
//!
//! - Data: the screener's session ([`Screener`]) given the markets of the
//!   list shown ([`WatchKey`]): it polls the statistics scopes of their
//!   exchanges and products only, and its table keeps the rows of those
//!   markets. A market is found by its exchange and catalog id, as the
//!   Funding Rates finds one.
//! - Settings ([`WatchlistSettings`]): the lists (a name and markets each,
//!   in order), the one shown, and the screener's column settings beside
//!   them in the same tab record, shared by every list. Its scope and
//!   products are never set: the markets choose what is polled.
//! - View: the lists' tabs over the screener's table ([`tabs`]: switch,
//!   `+ New List`, a ▾ menu per tab to rename or remove it, and a drag to
//!   move it),
//!   "Add pair" after the last row (it opens the market picker) and a × at
//!   the end of each row; a pick that cannot be added says why in a strip
//!   under the tabs. The pane's gear opens the screener's column list.

mod tabs;

use std::{collections::HashSet, rc::Rc};

use nsh_core::prelude::*;
use serde::Serialize;
use serde_json::Value;

use super::{
    panel,
    session::{PRODUCTS, Screener, WatchKey},
    settings::ScreenerSettings,
    table::{self, Editing},
};
use crate::{
    data::{DataServices, SessionScope, ferris::MarketSelection},
    theme,
    widgets::{
        Category, TabContext,
        common::{
            MarketPicker,
            access::{Voice, strip},
        },
        host::Widget,
    },
};

/// The longest name a list takes, in characters.
const NAME_LENGTH: usize = 32;

/// The Watchlist, as the registry lists it.
pub(crate) struct Watchlist;

impl Widget for Watchlist {
    const ID: &'static str = "watchlist";
    const TITLE: &'static str = "Watchlist";
    const DESCRIPTION: &'static str = "Track lists of markets you pick with the screener's prices, funding, volume and open interest.";
    const CATEGORY: Category = Category::Market;

    type Settings = WatchlistSettings;
    type Session = Screener;

    fn session(
        scope: SessionScope,
        services: &DataServices,
        settings: Signal<WatchlistSettings>,
    ) -> Screener {
        // The table's part of the settings, which the screener follows.
        let columns = signal(settings.with_untracked(|s| s.table.clone()));
        effect(move || {
            let next = settings.with(|s| s.table.clone());
            if columns.with_untracked(|current| *current != next) {
                columns.set(next);
            }
        });
        let watched = computed(move || settings.with(WatchlistSettings::keys));
        Screener::new(scope, services.ferris.clone(), columns, Some(watched))
    }

    fn view(screener: &Rc<Screener>, tab: &TabContext<WatchlistSettings>) -> View {
        let picking = signal(false);
        let refusal = signal(None::<Refused>);
        // A tab that is not shown has no picker open.
        let shown = tab.shown;
        effect(move || {
            if !shown.get() && picking.get_untracked() {
                picking.set(false);
            }
        });
        let add = Rc::new(move || {
            refusal.set(None);
            picking.set(true);
        });
        let remove = {
            let tab = tab.clone();
            Rc::new(move |key: WatchKey| {
                refusal.set(None);
                tab.save(&tab.settings.get_untracked().without(&key));
            })
        };
        let saved = tab.settings;
        let editing = Editing {
            bar: tabs::bar(tab.clone(), refusal),
            notes: notes(screener, refusal),
            add,
            remove,
            list: computed(move || saved.with(WatchlistSettings::active)),
        };
        let picker = {
            let tab = tab.clone();
            dynamic(move || picking.get().then(|| picker(tab.clone(), picking, refusal)))
        };
        table::table_view(screener.clone(), columns_of(tab), Some(editing))
            .test_id("watchlist")
            .child(picker)
    }

    fn panel(screener: &Rc<Screener>, tab: &TabContext<WatchlistSettings>) -> Option<Overlay> {
        Some(panel::panel(screener.clone(), columns_of(tab)))
    }
}

/// The tab seen as the screener's settings it holds.
fn columns_of(tab: &TabContext<WatchlistSettings>) -> TabContext<ScreenerSettings> {
    tab.project(|s| s.table.clone(), WatchlistSettings::with_table)
}

/// The strips under the tabs: why the last pick was not added, and how
/// many of the list's markets have no statistics while others do (with
/// none at all, the table says so).
fn notes(screener: &Screener, refusal: Signal<Option<Refused>>) -> View {
    let (table, watched) = (screener.table, screener.watched);
    let missing = computed(move || {
        let total = watched.map_or(0, |watched| watched.with(Vec::len));
        table.with(|table| {
            let table = &table.0;
            let settled = table.universe.answered && !table.universe.waiting;
            (settled && table.missing > 0 && table.missing < total).then_some(table.missing)
        })
    });
    view().flex_none().flex_col().child(dynamic(move || {
        let refused = refusal
            .get()
            .map(|refused| strip(refused.message(), Voice::Waiting));
        let missing = missing.get().map(|count| {
            let said = if count == 1 {
                "1 pair in this list has no statistics.".to_owned()
            } else {
                format!("{count} pairs in this list have no statistics.")
            };
            strip(said, Voice::Plain)
        });
        refused
            .into_iter()
            .chain(missing)
            .map(|strip| strip.border_b_1().border_color(theme::BORDER))
            .collect::<Vec<_>>()
    }))
}

/// The market picker, of perpetuals and futures: a pick is added, or the
/// widget says why not.
fn picker(
    tab: TabContext<WatchlistSettings>,
    picking: Signal<bool>,
    refusal: Signal<Option<Refused>>,
) -> View {
    MarketPicker::new(
        move |market| match tab.settings.get_untracked().adding(market) {
            Ok(next) => tab.save(&next),
            Err(refused) => refusal.set(Some(refused)),
        },
    )
    .products(&PRODUCTS)
    .view(move || picking.set(false))
}

/// Why a pick was not added, which the widget says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refused {
    /// Not a perpetual or future with a catalog id.
    Unsupported,
    /// Watched already.
    Duplicate,
}

impl Refused {
    pub const fn message(self) -> &'static str {
        match self {
            Self::Unsupported => "Only perpetuals and futures can be watched.",
            Self::Duplicate => "This market is already in the watchlist.",
        }
    }
}

/// One list: its name and its markets, each once, in the order they were
/// added.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct List {
    pub name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub markets: Vec<MarketSelection>,
}

impl List {
    /// An empty list named for being the `number`th.
    fn numbered(number: usize) -> Self {
        Self {
            name: format!("List {number}"),
            markets: Vec::new(),
        }
    }

    /// A saved list, read leniently: a name that is missing or blank is
    /// `List <number>`; a market that cannot be watched, or is there
    /// twice, is dropped.
    fn read(value: &Value, number: usize) -> Option<Self> {
        let record = value.as_object()?;
        let name = record
            .get("name")
            .and_then(Value::as_str)
            .and_then(clean_name)
            .unwrap_or_else(|| Self::numbered(number).name);
        let markets = match record.get("markets") {
            Some(Value::Array(list)) => read_markets(list),
            _ => Vec::new(),
        };
        Some(Self { name, markets })
    }

    fn keys(&self) -> Vec<WatchKey> {
        self.markets.iter().filter_map(WatchKey::of).collect()
    }
}

/// The watchable markets of a saved list, each once, in order.
fn read_markets(list: &[Value]) -> Vec<MarketSelection> {
    let mut seen = HashSet::new();
    MarketSelection::read_list(list)
        .into_iter()
        .filter(|market| WatchKey::of(market).is_some_and(|key| seen.insert(key)))
        .collect()
}

/// `name` trimmed and cut to [`NAME_LENGTH`]; `None` when blank.
fn clean_name(name: &str) -> Option<String> {
    let name: String = name.trim().chars().take(NAME_LENGTH).collect();
    let name = name.trim_end().to_owned();
    (!name.is_empty()).then_some(name)
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

/// One watchlist's settings: its lists (`lists`), the one shown
/// (`activeList`), and the screener's column settings under their own
/// names. Each is left out of the saved form while it holds its default
/// (one empty `List 1`, shown), and read leniently.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct WatchlistSettings {
    /// The lists, in their tabs' order; empty is one empty `List 1`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    lists: Vec<List>,
    /// The list shown, an index into `lists`.
    #[serde(rename = "activeList", skip_serializing_if = "is_zero")]
    active: usize,
    #[serde(flatten)]
    table: ScreenerSettings,
}

impl From<Value> for WatchlistSettings {
    fn from(value: Value) -> Self {
        let Value::Object(mut members) = value else {
            return Self::default();
        };
        let mut lists: Vec<List> = match members.remove("lists") {
            Some(Value::Array(saved)) => saved
                .iter()
                .enumerate()
                .filter_map(|(at, list)| List::read(list, at + 1))
                .collect(),
            _ => Vec::new(),
        };
        // The first Watchlist saved one list of `markets`.
        if let (true, Some(Value::Array(markets))) = (lists.is_empty(), members.remove("markets")) {
            lists.push(List {
                markets: read_markets(&markets),
                ..List::numbered(1)
            });
        }
        let active = members
            .remove("activeList")
            .and_then(|active| active.as_u64())
            .map_or(0, |active| active as usize);
        // The markets choose what is polled: a scope or products mean
        // nothing here.
        members.remove("scope");
        members.remove("visibleProducts");
        Self {
            lists: Vec::new(),
            active: 0,
            table: ScreenerSettings::from(Value::Object(members)),
        }
        .with_lists(lists, active)
    }
}

impl WatchlistSettings {
    /// The lists, in their tabs' order: never none.
    pub(super) fn lists(&self) -> Vec<List> {
        if self.lists.is_empty() {
            vec![List::numbered(1)]
        } else {
            self.lists.clone()
        }
    }

    /// The list shown, an index into [`Self::lists`].
    pub(super) fn active(&self) -> usize {
        self.active.min(self.lists.len().saturating_sub(1))
    }

    /// The keys of the list shown's markets, in order.
    fn keys(&self) -> Vec<WatchKey> {
        self.lists
            .get(self.active())
            .map_or_else(Vec::new, List::keys)
    }

    fn with_table(self, table: ScreenerSettings) -> Self {
        Self { table, ..self }
    }

    /// These settings with `lists` (one empty `List 1` when none), the
    /// one at `active` shown (the last when past them). One empty
    /// `List 1` shown is the default, saved as nothing.
    fn with_lists(self, mut lists: Vec<List>, active: usize) -> Self {
        if lists.is_empty() {
            lists.push(List::numbered(1));
        }
        let active = active.min(lists.len() - 1);
        let lists = if lists == [List::numbered(1)] {
            Vec::new()
        } else {
            lists
        };
        Self {
            lists,
            active,
            ..self
        }
    }

    /// The list shown changed by `change`.
    fn changing_shown(self, change: impl FnOnce(&mut List)) -> Self {
        let (mut lists, active) = (self.lists(), self.active());
        change(&mut lists[active]);
        self.with_lists(lists, active)
    }

    /// These settings showing the list at `at`.
    pub(super) fn select(self, at: usize) -> Self {
        let lists = self.lists();
        self.with_lists(lists, at)
    }

    /// These settings with a new empty list last, shown, named for how
    /// many lists there are with it (`List 3` when there were two).
    pub(super) fn with_new_list(self) -> Self {
        let mut lists = self.lists();
        lists.push(List::numbered(lists.len() + 1));
        let last = lists.len() - 1;
        self.with_lists(lists, last)
    }

    /// The list at `at` named `name` (trimmed and cut short); unchanged
    /// when `name` is blank.
    pub(super) fn renamed(self, at: usize, name: &str) -> Self {
        let (mut lists, active) = (self.lists(), self.active());
        match (clean_name(name), lists.get_mut(at)) {
            (Some(name), Some(list)) => {
                list.name = name;
                self.with_lists(lists, active)
            }
            _ => self,
        }
    }

    /// The list at `at` moved `by` places, the one shown kept shown;
    /// `None` when it cannot move that far.
    pub(super) fn moved(self, at: usize, by: isize) -> Option<Self> {
        let (mut lists, active) = (self.lists(), self.active());
        let to = at.checked_add_signed(by).filter(|&to| to < lists.len())?;
        if at >= lists.len() || to == at {
            return None;
        }
        let list = lists.remove(at);
        lists.insert(to, list);
        let active = if active == at {
            to
        } else if at < active && active <= to {
            active - 1
        } else if to <= active && active < at {
            active + 1
        } else {
            active
        };
        Some(self.with_lists(lists, active))
    }

    /// These settings without the list at `at`, the one shown kept shown
    /// (its neighbour when it goes); `None` for the last list left.
    pub(super) fn removed(self, at: usize) -> Option<Self> {
        let (mut lists, active) = (self.lists(), self.active());
        if lists.len() < 2 || at >= lists.len() {
            return None;
        }
        lists.remove(at);
        let active = if at < active { active - 1 } else { active };
        Some(self.with_lists(lists, active))
    }

    /// These settings with `market` last in the list shown.
    pub(super) fn adding(self, market: MarketSelection) -> Result<Self, Refused> {
        let key = WatchKey::of(&market).ok_or(Refused::Unsupported)?;
        if self.keys().contains(&key) {
            return Err(Refused::Duplicate);
        }
        Ok(self.changing_shown(|list| list.markets.push(market)))
    }

    /// These settings without the market `key` names in the list shown.
    pub(super) fn without(self, key: &WatchKey) -> Self {
        self.changing_shown(|list| {
            list.markets.retain(|market| {
                WatchKey::of(market).is_none_or(|kept| {
                    kept.exchange != key.exchange || kept.market_id != key.market_id
                })
            });
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::data::ferris::{MarketType, Product};

    fn market(exchange: &str, id: &str, kind: MarketType) -> MarketSelection {
        MarketSelection {
            exchange: exchange.into(),
            market_id: Some(id.into()),
            kind,
            ..MarketSelection::bybit_btc_usdt()
        }
    }

    impl WatchlistSettings {
        /// The markets of the list shown, in the order they were added.
        fn markets(&self) -> Vec<MarketSelection> {
            self.lists()[self.active()].markets.clone()
        }
    }

    fn names(settings: &WatchlistSettings) -> Vec<String> {
        settings.lists().into_iter().map(|list| list.name).collect()
    }

    #[test]
    fn markets_are_added_once_and_only_perpetuals_and_futures() {
        let settings = WatchlistSettings::default()
            .adding(market("bybit", "1", MarketType::Perp))
            .unwrap()
            .adding(market("binance", "1", MarketType::Future))
            .unwrap();
        assert_eq!(settings.markets().len(), 2);
        assert_eq!(
            settings
                .clone()
                .adding(market("bybit", "1", MarketType::Perp)),
            Err(Refused::Duplicate)
        );
        assert_eq!(
            settings
                .clone()
                .adding(market("bybit", "2", MarketType::Spot)),
            Err(Refused::Unsupported)
        );
        let unlisted = MarketSelection {
            market_id: None,
            ..market("bybit", "3", MarketType::Perp)
        };
        assert_eq!(settings.adding(unlisted), Err(Refused::Unsupported));
    }

    #[test]
    fn removing_a_market_keeps_the_others_in_order() {
        let settings = WatchlistSettings::default()
            .adding(market("bybit", "1", MarketType::Perp))
            .unwrap()
            .adding(market("okx", "2", MarketType::Perp))
            .unwrap()
            .adding(market("binance", "3", MarketType::Perp))
            .unwrap();
        let key = WatchKey {
            exchange: "okx".into(),
            market_id: "2".into(),
            product: Product::Perp,
        };
        let ids: Vec<_> = settings
            .without(&key)
            .markets()
            .iter()
            .map(|m| m.market_id.clone().unwrap())
            .collect();
        assert_eq!(ids, ["1", "3"]);
    }

    #[test]
    fn new_lists_are_numbered_by_how_many_there_are_and_shown() {
        let settings = WatchlistSettings::default().with_new_list().with_new_list();
        assert_eq!(names(&settings), ["List 1", "List 2", "List 3"]);
        assert_eq!(settings.active(), 2);
        // Removing one does not renumber the others; the next is named for
        // the count.
        let settings = settings.removed(0).unwrap().with_new_list();
        assert_eq!(names(&settings), ["List 2", "List 3", "List 3"]);
    }

    #[test]
    fn lists_keep_their_own_markets() {
        let settings = WatchlistSettings::default()
            .adding(market("bybit", "1", MarketType::Perp))
            .unwrap()
            .with_new_list()
            .adding(market("okx", "2", MarketType::Perp))
            .unwrap();
        assert_eq!(settings.markets()[0].exchange, "okx");
        let first = settings.select(0);
        assert_eq!(first.markets().len(), 1);
        assert_eq!(first.markets()[0].exchange, "bybit");
        // The same market may be in two lists.
        assert!(
            first
                .with_new_list()
                .adding(market("bybit", "1", MarketType::Perp))
                .is_ok()
        );
    }

    #[test]
    fn renaming_trims_cuts_and_ignores_blank_names() {
        let settings = WatchlistSettings::default().renamed(0, "  Majors  ");
        assert_eq!(names(&settings), ["Majors"]);
        assert_eq!(names(&settings.clone().renamed(0, "   ")), ["Majors"]);
        let long = settings.renamed(0, &"x".repeat(50));
        assert_eq!(long.lists()[0].name.len(), NAME_LENGTH);
    }

    #[test]
    fn moving_a_list_keeps_the_one_shown() {
        let settings = WatchlistSettings::default()
            .with_new_list()
            .with_new_list()
            .select(0);
        let moved = settings.clone().moved(0, 1).unwrap();
        assert_eq!(names(&moved), ["List 2", "List 1", "List 3"]);
        assert_eq!(moved.active(), 1);
        let other = settings.clone().moved(2, -1).unwrap();
        assert_eq!(names(&other), ["List 1", "List 3", "List 2"]);
        assert_eq!(other.active(), 0);
        assert_eq!(settings.clone().moved(0, -1), None);
        assert_eq!(settings.moved(2, 1), None);
    }

    #[test]
    fn removing_a_list_keeps_the_one_shown_and_never_the_last() {
        let settings = WatchlistSettings::default().with_new_list().with_new_list();
        let removed = settings.clone().removed(0).unwrap();
        assert_eq!(names(&removed), ["List 2", "List 3"]);
        assert_eq!(removed.active(), 1);
        let shown = settings.removed(2).unwrap();
        assert_eq!(shown.active(), 1);
        assert_eq!(WatchlistSettings::default().removed(0), None);
    }

    #[test]
    fn settings_round_trip_and_read_leniently() {
        let settings = WatchlistSettings::default()
            .adding(market("bybit", "1", MarketType::Perp))
            .unwrap()
            .with_new_list()
            .renamed(1, "Alts");
        let saved = serde_json::to_value(&settings).unwrap();
        assert_eq!(WatchlistSettings::from(saved), settings);
        assert_eq!(
            serde_json::to_value(WatchlistSettings::default()).unwrap(),
            json!({})
        );
        // Back to one empty List 1, shown: the default again.
        let back = settings.removed(1).unwrap().without(&WatchKey {
            exchange: "bybit".into(),
            market_id: "1".into(),
            product: Product::Perp,
        });
        assert_eq!(back, WatchlistSettings::default());

        let perp = serde_json::to_value(market("bybit", "1", MarketType::Perp)).unwrap();
        let spot = serde_json::to_value(market("bybit", "2", MarketType::Spot)).unwrap();
        let read = WatchlistSettings::from(json!({
            "lists": [
                {"name": " ", "markets": [perp.clone(), perp.clone(), spot, "junk"]},
                "junk",
                {"name": "B"},
            ],
            "activeList": 9,
            "scope": "bybit:linear-perp",
            "visibleProducts": ["future"],
        }));
        assert_eq!(names(&read), ["List 1", "B"]);
        assert_eq!(read.active(), 1);
        assert_eq!(read.clone().select(0).markets().len(), 1);
        assert_eq!(read.table, ScreenerSettings::default());
        // The first Watchlist's one list of markets.
        let old = WatchlistSettings::from(json!({ "markets": [perp] }));
        assert_eq!(names(&old), ["List 1"]);
        assert_eq!(old.markets().len(), 1);
        assert_eq!(
            WatchlistSettings::from(json!("junk")),
            WatchlistSettings::default()
        );
    }
}
