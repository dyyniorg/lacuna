//! Minimal review state kept between cells.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::Municipality;
use crate::utils;

use super::ReviewError;

/// Missing building refs and round identity for one review run.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReviewState {
    pub round_id: String,
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
    utils::save_json(&path(cache_dir, municipality), state, true)?;
    Ok(())
}
