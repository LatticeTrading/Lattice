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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_round_trips_layout_and_repairs_ids() {
        let mut grid = Grid::default();
        grid.spawn();
        grid.add_tab(1);
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
        assert_eq!(restored.pages[0].grid.spawn(), 2);
    }
}
