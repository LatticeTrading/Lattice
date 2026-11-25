//! Versioned workspace persistence through NSH's platform storage service.
//!
//! NSH maps this service to the native app data directory on desktop and to
//! browser storage on WebAssembly, so the same Rust persistence boundary works
//! on both targets.

use anyhow::{Context as _, Result};
use nsh_core::storage::Storage;
use serde::{Deserialize, Serialize};

use crate::layout::Grid;

const STORAGE_VERSION: u32 = 1;
const STORAGE_KEY: &str = "lattice.workspace.v1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct StoredPage {
    pub id: usize,
    pub grid: Grid,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct StoredWorkspace {
    pub version: u32,
    pub active_page_id: usize,
    pub next_page_id: usize,
    pub pages: Vec<StoredPage>,
}

impl Default for StoredWorkspace {
    fn default() -> Self {
        Self {
            version: STORAGE_VERSION,
            active_page_id: 1,
            next_page_id: 2,
            pages: Vec::new(),
        }
    }
}

/// Load the last workspace. Missing, old, or malformed state opens a fresh
/// workspace rather than preventing the app from starting.
pub(crate) fn load() -> StoredWorkspace {
    let Some(contents) = Storage::get_item(STORAGE_KEY) else {
        return StoredWorkspace::default();
    };
    match serde_json::from_str::<StoredWorkspace>(&contents) {
        Ok(workspace) if workspace.version == STORAGE_VERSION => workspace,
        Ok(workspace) => {
            log::warn!("Lattice: ignoring workspace version {}", workspace.version);
            StoredWorkspace::default()
        }
        Err(error) => {
            log::warn!("Lattice: ignoring malformed workspace: {error}");
            StoredWorkspace::default()
        }
    }
}

pub(crate) fn save(workspace: &StoredWorkspace) -> Result<()> {
    let contents = serde_json::to_string(workspace).context("serializing workspace")?;
    Storage::set_item(STORAGE_KEY, contents).context("saving workspace")?;
    Ok(())
}

const SETTINGS_KEY: &str = "lattice.settings.v1";

/// The user's preferences from Settings. A field added later takes its
/// default, so earlier saves keep loading.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct StoredSettings {
    /// The chosen theme's id (its file name in `themes/`).
    pub theme: Option<String>,
    /// How the Everything Bar's clock reads.
    pub clock: StoredClock,
    /// The Everything Bar's link color id (`blue`): which widgets its
    /// search sends a pick to. Absent: the default color.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_color: Option<String>,
    /// How notifications pop up.
    pub notifications: StoredNotifications,
}

/// The notifications' saved preferences (`crate::notify::Prefs`): popups
/// on, in the default corner, until the user says otherwise.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct StoredNotifications {
    /// Whether a notification pops up in a corner of the window as it
    /// arrives. The bell's list keeps it either way.
    pub popups: bool,
    /// The corner's id (`bottom-right`). Absent, or one no longer
    /// offered: the default corner.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corner: Option<String>,
}

impl Default for StoredNotifications {
    fn default() -> Self {
        Self {
            popups: true,
            corner: None,
        }
    }
}

/// The clock's saved preferences: 24-hour UTC+0 by default.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct StoredClock {
    pub hour12: bool,
    /// Minutes east of UTC (UTC-6 is -360).
    pub utc_offset_minutes: i16,
}

/// Load the saved preferences; missing or malformed ones are the defaults.
pub(crate) fn load_settings() -> StoredSettings {
    let Some(contents) = Storage::get_item(SETTINGS_KEY) else {
        return StoredSettings::default();
    };
    serde_json::from_str(&contents).unwrap_or_else(|error| {
        log::warn!("Lattice: ignoring malformed settings: {error}");
        StoredSettings::default()
    })
}

pub(crate) fn save_settings(settings: &StoredSettings) -> Result<()> {
    let contents = serde_json::to_string(settings).context("serializing settings")?;
    Storage::set_item(SETTINGS_KEY, contents).context("saving settings")?;
    Ok(())
}

const ADDRESS_GROUPS_KEY: &str = "lattice.address-groups.v1";

/// The saved address groups as they were written, which their model reads
/// leniently (`crate::widgets::common::accounts`); `None` when none are
/// saved or what is saved is not JSON.
pub(crate) fn load_address_groups() -> Option<serde_json::Value> {
    let contents = Storage::get_item(ADDRESS_GROUPS_KEY)?;
    serde_json::from_str(&contents)
        .map_err(|error| log::warn!("Lattice: ignoring malformed address groups: {error}"))
        .ok()
}

pub(crate) fn save_address_groups(groups: &impl Serialize) -> Result<()> {
    let contents = serde_json::to_string(groups).context("serializing address groups")?;
    Storage::set_item(ADDRESS_GROUPS_KEY, contents).context("saving address groups")?;
    Ok(())
}

/// Where what `venue` keeps to trade between runs is saved.
fn trading_key(venue: &str) -> String {
    format!("lattice.trading.{venue}.v1")
}

/// What the exchange `venue` keeps to trade for the user's accounts between
/// runs, as its integration wrote it (`crate::data::trading`); `None` when
/// it keeps nothing or what is saved is not JSON.
///
/// It is the one thing saved that is a secret: keys that can trade, and
/// cannot send an account's funds anywhere but to the wallet that owns it,
/// in the clear, as everything this storage holds is. An integration saves
/// only what its exchange lets it revoke.
pub(crate) fn load_trading_keys(venue: &str) -> Option<serde_json::Value> {
    let contents = Storage::get_item(&trading_key(venue))?;
    serde_json::from_str(&contents)
        .map_err(|error| log::warn!("Lattice: ignoring malformed {venue} trading keys: {error}"))
        .ok()
}

/// Saves what `venue` keeps to trade between runs; `None` keeps nothing
/// (the entry goes).
pub(crate) fn save_trading_keys(venue: &str, keys: Option<&serde_json::Value>) -> Result<()> {
    let Some(keys) = keys else {
        Storage::remove_item(&trading_key(venue));
        return Ok(());
    };
    let contents = serde_json::to_string(keys).context("serializing trading keys")?;
    Storage::set_item(trading_key(venue), contents).context("saving trading keys")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_round_trips_layout_and_repairs_ids() {
        let mut grid = Grid::default();
        grid.spawn(crate::widgets::WidgetKind::MarketScreener);
        grid.add_tab(1, crate::widgets::WidgetKind::MarketScreener);
        let storage = StoredWorkspace {
            pages: vec![StoredPage { id: 1, grid }],
            ..StoredWorkspace::default()
        };
        let json = serde_json::to_string(&storage).expect("serialize workspace");
        let mut restored: StoredWorkspace =
            serde_json::from_str(&json).expect("deserialize workspace");
        restored.pages[0].grid.repair_counters();
        assert_eq!(restored.version, STORAGE_VERSION);
        assert_eq!(restored.pages[0].grid.layouts[4][0].tabs.len(), 2);
        assert_eq!(
            restored.pages[0]
                .grid
                .spawn(crate::widgets::WidgetKind::MarketScreener),
            2
        );
    }
}
