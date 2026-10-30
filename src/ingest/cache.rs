//! Cache layout and cache-state inspection.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::Value;

use crate::config::Municipality;
use crate::utils;

use super::error::IngestError;
use super::report::Stage;

/// The cache layout used by the ingest client.
#[derive(Clone, Debug)]
pub struct Cache {
    root: PathBuf,
}

impl Cache {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn ensure_root(&self) -> Result<(), IngestError> {
        fs::create_dir_all(&self.root)?;
        Ok(())
    }

    pub fn pbf_path(&self) -> PathBuf {
        self.root.join("finland_latest.osm.pbf")
    }

    pub fn municipality_dir(&self, municipality: &Municipality) -> PathBuf {
        self.root.join(municipality.slug())
    }

    pub fn municipality_file(&self, municipality: &Municipality, name: &str) -> PathBuf {
        self.municipality_dir(municipality).join(name)
    }

    /// Cached artifact path for one ingest stage.
    pub fn path(&self, municipality: &Municipality, stage: Stage) -> PathBuf {
        match stage.file_name() {
            Some(name) => self.municipality_file(municipality, name),
            None => self.pbf_path(),
        }
    }

    pub fn load_json(&self, path: &Path) -> Result<Option<Value>, IngestError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };

        let value = serde_json::from_str(&text).map_err(|error| IngestError::InvalidCache {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;

        Ok(Some(value))
    }

    pub fn save_json(&self, path: &Path, value: &Value) -> Result<(), IngestError> {
        utils::save_json(path, value, false)?;
        Ok(())
    }

    /// Returns true when `path` exists and is at least as new as `than`.
    pub fn is_newer(&self, path: &Path, than: &Path) -> Result<bool, IngestError> {
        let modified = fs::metadata(path).and_then(|metadata| metadata.modified());
        let reference = fs::metadata(than).and_then(|metadata| metadata.modified());

        match (modified, reference) {
            (Ok(modified), Ok(reference)) => Ok(modified >= reference),
            _ => Ok(false),
        }
    }

    /// Returns true when `path` exists and is younger than `max_age`.
    pub fn is_fresh(&self, path: &Path, max_age: Duration) -> Result<bool, IngestError> {
        Ok(self.age(path)?.is_some_and(|age| age < max_age))
    }

    /// Returns the time since `path` was last modified.
    pub fn age(&self, path: &Path) -> Result<Option<Duration>, IngestError> {
        Ok(metadata(path)?.and_then(|metadata| age_of(&metadata)))
    }

    fn entry(&self, kind: Stage, path: PathBuf, max_age: Duration) -> Result<CacheEntry, IngestError> {
        let (exists, size_bytes, age) = match metadata(&path)? {
            Some(metadata) => (true, metadata.len(), age_of(&metadata)),
            None => (false, 0, None),
        };
        let fresh = age.is_some_and(|age| age < max_age);

        Ok(CacheEntry {
            kind,
            path,
            exists,
            size_bytes,
            age,
            fresh,
        })
    }

    /// Current state of all cached ingest artifacts for one municipality.
    pub fn state(&self, municipality: &Municipality, max_age: Duration) -> Result<CacheState, IngestError> {
        let stages = [
            Stage::OsmExtract,
            Stage::RyhtiBuildings,
            Stage::RyhtiAddresses,
            Stage::NlsFootprints,
            Stage::Parcels,
            Stage::OsmBuildings,
        ];

        let entries = stages
            .into_iter()
            .map(|stage| self.entry(stage, self.path(municipality, stage), max_age))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(CacheState { entries })
    }
}

fn metadata(path: &Path) -> Result<Option<fs::Metadata>, IngestError> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn age_of(metadata: &fs::Metadata) -> Option<Duration> {
    let modified = metadata.modified().ok()?;
    SystemTime::now().duration_since(modified).ok()
}

/// Current state of one cached artifact.
#[derive(Clone, Debug)]
pub struct CacheEntry {
    pub kind: Stage,
    pub path: PathBuf,
    pub exists: bool,
    pub size_bytes: u64,
    /// Time since the file was last modified.
    pub age: Option<Duration>,
    /// Whether the artifact is within the configured maximum age.
    pub fresh: bool,
}

/// Current state of all cached artifacts.
#[derive(Clone, Debug)]
pub struct CacheState {
    pub entries: Vec<CacheEntry>,
}
