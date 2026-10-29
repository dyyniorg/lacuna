//! Persistent per-cell review progress.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config::Municipality;

/// Review status for one grid cell.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CellProgress {
    #[serde(default)]
    pub buildings: bool,
    #[serde(default)]
    pub addresses: bool,
}

/// Progress for one full review round.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReviewProgress {
    #[serde(default)]
    pub round_id: String,
    #[serde(default)]
    pub cells: BTreeMap<String, CellProgress>,
    #[serde(default)]
    pub notes: String,
}

impl ReviewProgress {
    pub fn new(cells: &[String]) -> Self {
        Self::with_round_id(cells, now_id())
    }

    pub fn with_round_id(cells: &[String], round_id: String) -> Self {
        Self {
            round_id,
            cells: cells
                .iter()
                .map(|cell| (cell.clone(), CellProgress::default()))
                .collect(),
            notes: String::new(),
        }
    }

    /// Keeps only current cells and adds missing ones.
    pub fn sync_cells(&mut self, cells: &[String]) {
        self.cells.retain(|number, _| cells.iter().any(|cell| cell == number));

        for cell in cells {
            self.cells.entry(cell.clone()).or_insert_with(CellProgress::default);
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

        if !entry.buildings {
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
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let text = serde_json::to_string_pretty(progress).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.tmp");

    fs::write(&temporary, text).map_err(|error| error.to_string())?;
    fs::rename(&temporary, path).map_err(|error| error.to_string())?;

    Ok(())
}

fn now_id() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "lacuna-progress-test-{label}-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));

        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("failed to create test directory");

        path
    }

    #[test]
    fn progress_round_trip_and_rules() {
        let dir = temp_dir("round-trip");
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

        progress.notes = "review notes".to_string();
        save(&path, &progress).unwrap();

        let loaded = load(&path).unwrap().expect("progress file missing");
        assert_eq!(loaded, progress);

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"buildings\""));
        assert!(text.contains("\"addresses\""));
        assert!(text.contains("review notes"));

        let _ = fs::remove_dir_all(&dir);
    }
}
