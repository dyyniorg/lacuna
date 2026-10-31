//! Persistent per-cell review progress.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::Municipality;
use crate::utils;

/// Review status for one grid cell.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CellProgress {
    pub buildings: bool,
    pub addresses: bool,
    /// No building proposals exist for this cell.
    #[serde(default)]
    pub skipped: bool,
}

/// Progress for one full review round.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReviewProgress {
    pub round_id: String,
    pub cells: BTreeMap<String, CellProgress>,
}

impl ReviewProgress {
    pub fn new(cells: &[String]) -> Self {
        Self::with_round_id(cells, utils::now_id())
    }

    pub fn with_round_id(cells: &[String], round_id: String) -> Self {
        Self {
            round_id,
            cells: cells
                .iter()
                .map(|cell| (cell.clone(), CellProgress::default()))
                .collect(),
        }
    }

    /// Keeps only current cells and adds missing ones.
    pub fn sync_cells(&mut self, cells: &[String]) {
        self.cells.retain(|number, _| cells.iter().any(|cell| cell == number));

        for cell in cells {
            self.cells.entry(cell.clone()).or_default();
        }
    }

    pub fn set_buildings(&mut self, cell: &str) -> Result<(), String> {
        let entry = self
            .cells
            .get_mut(cell)
            .ok_or_else(|| format!("Unknown review cell: {cell}"))?;

        entry.buildings = true;

        Ok(())
    }

    pub fn set_addresses(&mut self, cell: &str) -> Result<(), String> {
        let entry = self
            .cells
            .get_mut(cell)
            .ok_or_else(|| format!("Unknown review cell: {cell}"))?;

        if !entry.buildings && !entry.skipped {
            return Err("Buildings must be reviewed before marking addresses done".to_string());
        }

        entry.addresses = true;

        Ok(())
    }
}

pub fn path(cache_dir: &Path, municipality: &Municipality) -> PathBuf {
    cache_dir.join(municipality.slug()).join("progress.json")
}

pub fn load(path: &Path) -> Result<Option<ReviewProgress>, String> {
    if !path.exists() {
        return Ok(None);
    }

    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let progress = serde_json::from_str(&text).map_err(|error| error.to_string())?;

    Ok(Some(progress))
}

pub fn save(path: &Path, progress: &ReviewProgress) -> Result<(), String> {
    utils::save_json(path, progress, true).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::utils::test_support::TempDir;

    use super::*;

    #[test]
    fn progress_round_trip_and_rules() {
        let dir = TempDir::new("progress", "round-trip");
        let path = dir.join("progress.json");

        let mut progress = ReviewProgress::new(&["01_00".to_string(), "02_00".to_string()]);
        assert!(!progress.round_id.is_empty());
        assert!(progress.cells.contains_key("01_00"));
        assert!(!progress.cells["01_00"].buildings);
        assert!(progress.set_addresses("01_00").is_err());

        progress.set_buildings("01_00").unwrap();
        progress.set_addresses("01_00").unwrap();
        assert!(progress.cells["01_00"].buildings);
        assert!(progress.cells["01_00"].addresses);

        progress.sync_cells(&["02_00".to_string(), "03_00".to_string()]);
        assert!(!progress.cells.contains_key("01_00"));
        assert!(progress.cells.contains_key("02_00"));
        assert!(progress.cells.contains_key("03_00"));
        assert!(!progress.cells["02_00"].buildings);

        save(&path, &progress).unwrap();

        let loaded = load(&path).unwrap().expect("progress file missing");
        assert_eq!(loaded, progress);

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"buildings\""));
        assert!(text.contains("\"addresses\""));
        assert!(text.contains("\"skipped\""));
    }

    #[test]
    fn skipped_cells_can_mark_addresses_done() {
        let mut progress = ReviewProgress::new(&["01_00".to_string()]);
        progress.cells.get_mut("01_00").unwrap().skipped = true;

        progress.set_addresses("01_00").unwrap();

        assert!(progress.cells["01_00"].addresses);
        assert!(!progress.cells["01_00"].buildings);
    }
}
