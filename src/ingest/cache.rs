//! Cache layout and cache-state inspection.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::Value;

use crate::config::Municipality;

use super::error::IngestError;

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
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let text = serde_json::to_string(value)?;
        fs::write(path, text)?;

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
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };

        let age = metadata
            .modified()
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok());

        Ok(age.map(|age| age < max_age).unwrap_or(false))
    }

    pub fn entry(&self, kind: CacheKind, path: PathBuf, max_age: Duration) -> Result<CacheEntry, IngestError> {
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };

        let (exists, size_bytes, age) = match metadata {
            Some(metadata) => {
                let age = metadata
                    .modified()
                    .ok()
                    .and_then(|modified| SystemTime::now().duration_since(modified).ok());
                (true, metadata.len(), age)
            }
            None => (false, 0, None),
        };

        let fresh = age.map(|age| age < max_age).unwrap_or(false);

        Ok(CacheEntry {
            kind,
            path,
            exists,
            size_bytes,
            age,
            fresh,
        })
    }
}

/// One cached input artifact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheKind {
    PbfExtract,
    RyhtiBuildings,
    RyhtiAddresses,
    NlsFootprints,
    Parcels,
    OsmBuildings,
}

impl CacheKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::PbfExtract => "Finland OSM extract",
            Self::RyhtiBuildings => "Ryhti buildings",
            Self::RyhtiAddresses => "Ryhti addresses",
            Self::NlsFootprints => "NLS footprints",
            Self::Parcels => "Land parcels",
            Self::OsmBuildings => "OSM buildings",
        }
    }
}

/// Current state of one cached artifact.
#[derive(Clone, Debug)]
pub struct CacheEntry {
    pub kind: CacheKind,
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
