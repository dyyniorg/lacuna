//! Minimal review state kept between cells.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config::Municipality;

use super::ReviewError;

/// Missing building refs and round identity for one review run.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ReviewState {
    #[serde(default)]
    pub round_id: String,
    #[serde(default)]
    pub missing_by_cell: BTreeMap<String, Vec<String>>,
}

pub fn path(cache_dir: &Path, municipality: &Municipality) -> PathBuf {
    cache_dir.join(municipality.slug()).join("review_state.json")
}

pub fn load(cache_dir: &Path, municipality: &Municipality) -> Result<Option<ReviewState>, ReviewError> {
    let path = path(cache_dir, municipality);

    if !path.exists() {
        return Ok(None);
    }

    let text = fs::read_to_string(path)?;
    let state = serde_json::from_str(&text)?;

    Ok(Some(state))
}

pub fn save(cache_dir: &Path, municipality: &Municipality, state: &ReviewState) -> Result<(), ReviewError> {
    let path = path(cache_dir, municipality);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let text = serde_json::to_string_pretty(state)?;
    let temporary = path.with_extension("json.tmp");

    fs::write(&temporary, text)?;
    fs::rename(&temporary, &path)?;

    Ok(())
}

pub fn now_id() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_default()
}
