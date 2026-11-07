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
