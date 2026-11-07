//! Durable local workspace storage.
//!
//! The persisted file contains only layout data. Widget implementations and
//! runtime state are intentionally not part of this boundary, so a future
//! widget can be replaced without making old layouts unreadable.

#[cfg(not(test))]
use std::{fs, io, path::PathBuf};

use serde::{Deserialize, Serialize};

use anyhow::Result;
#[cfg(not(test))]
use anyhow::{Context as _, anyhow};

use crate::layout::Grid;

const STORAGE_VERSION: u32 = 1;
#[cfg(not(test))]
const STORAGE_DIRECTORY: &str = "Lattice";
#[cfg(not(test))]
const STORAGE_FILE: &str = "workspace.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct StoredPage {
    pub id: usize,
    pub grid: Grid,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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

#[cfg(not(test))]
fn storage_path() -> Option<PathBuf> {
    dirs::data_local_dir().map(|directory| directory.join(STORAGE_DIRECTORY).join(STORAGE_FILE))
}

/// Load the last workspace. A missing, old, or malformed file is treated as a
/// fresh workspace rather than preventing the terminal from opening.
#[cfg(test)]
pub(crate) fn load() -> StoredWorkspace {
    StoredWorkspace::default()
}

#[cfg(not(test))]
pub(crate) fn load() -> StoredWorkspace {
    let Some(path) = storage_path() else {
        return StoredWorkspace::default();
    };

    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return StoredWorkspace::default();
        }
        Err(error) => {
            eprintln!(
                "Lattice: unable to read workspace {}: {error}",
                path.display()
            );
            return StoredWorkspace::default();
        }
    };

    match serde_json::from_str::<StoredWorkspace>(&contents) {
        Ok(workspace) if workspace.version == STORAGE_VERSION => workspace,
        Ok(workspace) => {
            eprintln!(
                "Lattice: ignoring unsupported workspace version {}",
                workspace.version
            );
            StoredWorkspace::default()
        }
        Err(error) => {
            eprintln!("Lattice: ignoring malformed workspace: {error}");
            StoredWorkspace::default()
        }
    }
}

/// Save with a same-directory temporary file so an interrupted write does not
/// leave a half-written workspace behind.
#[cfg(test)]
pub(crate) fn save(_: &StoredWorkspace) -> Result<()> {
    Ok(())
}

#[cfg(not(test))]
pub(crate) fn save(workspace: &StoredWorkspace) -> Result<()> {
    let path = storage_path().ok_or_else(|| anyhow!("no local application data directory"))?;
    let directory = path
        .parent()
        .ok_or_else(|| anyhow!("workspace path has no parent directory"))?;
    fs::create_dir_all(directory)
        .with_context(|| format!("creating workspace directory {}", directory.display()))?;

    let temporary = path.with_extension("json.tmp");
    let contents = serde_json::to_vec_pretty(workspace).context("serializing workspace")?;
    fs::write(&temporary, contents)
        .with_context(|| format!("writing temporary workspace {}", temporary.display()))?;
    if let Err(rename_error) = fs::rename(&temporary, &path) {
        // `rename` replaces an existing destination on Unix, but Windows
        // rejects that operation. Retry as a replacement there without
        // swallowing unrelated errors from the first rename.
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("removing old workspace {}", path.display()))?;
            if let Err(error) = fs::rename(&temporary, &path) {
                let _ = fs::remove_file(&temporary);
                return Err(error)
                    .with_context(|| format!("replacing workspace {}", path.display()));
            }
        } else {
            let _ = fs::remove_file(&temporary);
            return Err(rename_error)
                .with_context(|| format!("replacing workspace {}", path.display()));
        }
    }
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
