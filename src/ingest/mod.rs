//! Input data ingestion for the local review tool.
//!
//! The client fetches Ryhti buildings and addresses, NLS (MML) footprints, land
//! parcels, and OSM buildings from a fresh Finland PBF extract. Responses are cached
//! under the configured cache directory.

mod cache;
mod error;
mod geo;
mod http;
mod nls;
mod osm_pbf;
mod parcels;
mod report;
mod ryhti;

use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use crate::config::Municipality;
use crate::utils;

pub use cache::{Cache, CacheKind, CacheState};
pub use error::IngestError;
pub use geo::{BBox, bbox_of_points};
pub use report::{Progress, Reporter, Stage};

/// All input data needed by the review stage.
#[derive(Debug)]
pub struct IngestData {
    pub official: Value,
    pub addresses: Value,
    pub nls: Value,
    pub parcels: Value,
    pub osm: Value,
    pub bbox: BBox,
}

/// Fetches and caches all external input data.
pub struct IngestClient {
    http: reqwest::blocking::Client,
    download_http: reqwest::blocking::Client,
    cache: Cache,
    cache_max_age: Duration,
    reporter: Reporter,
}

impl IngestClient {
    pub fn new(params: &utils::Parameters) -> Result<Self, IngestError> {
        let cache = Cache::new(params.cache_dir().to_path_buf());
        cache.ensure_root()?;

        Ok(Self {
            http: http::api_client()?,
            download_http: http::download_client()?,
            cache,
            cache_max_age: params.cache_max_age(),
            reporter: Reporter::default(),
        })
    }

    pub fn with_reporter(mut self, reporter: Reporter) -> Self {
        self.reporter = reporter;
        self
    }

    pub fn cache_state(&self, municipality: &Municipality) -> Result<CacheState, IngestError> {
        let entries = vec![
            self.cache
                .entry(CacheKind::PbfExtract, self.cache.pbf_path(), self.cache_max_age)?,
            self.cache.entry(
                CacheKind::RyhtiBuildings,
                self.cache.municipality_file(municipality, "ryhti_buildings.json"),
                self.cache_max_age,
            )?,
            self.cache.entry(
                CacheKind::RyhtiAddresses,
                self.cache.municipality_file(municipality, "ryhti_addresses.json"),
                self.cache_max_age,
            )?,
            self.cache.entry(
                CacheKind::NlsFootprints,
                self.cache.municipality_file(municipality, "nls_footprints.json"),
                self.cache_max_age,
            )?,
            self.cache.entry(
                CacheKind::Parcels,
                self.cache.municipality_file(municipality, "parcels.json"),
                self.cache_max_age,
            )?,
            self.cache.entry(
                CacheKind::OsmBuildings,
                self.cache.municipality_file(municipality, "osm_buildings.json"),
                self.cache_max_age,
            )?,
        ];

        Ok(CacheState { entries })
    }

    pub fn ryhti_buildings(&self, municipality: &Municipality) -> Result<Value, IngestError> {
        let path = self.cache.municipality_file(municipality, "ryhti_buildings.json");
        self.cached_json(&path, Stage::RyhtiBuildings, "Ryhti buildings", || {
            ryhti::buildings(&self.http, municipality, &self.reporter)
        })
    }

    pub fn ryhti_addresses(&self, municipality: &Municipality) -> Result<Value, IngestError> {
        let path = self.cache.municipality_file(municipality, "ryhti_addresses.json");
        self.cached_json(&path, Stage::RyhtiAddresses, "Ryhti addresses", || {
            ryhti::addresses(&self.http, municipality, &self.reporter)
        })
    }

    pub fn nls_footprints(&self, municipality: &Municipality, bbox: &BBox) -> Result<Value, IngestError> {
        let path = self.cache.municipality_file(municipality, "nls_footprints.json");
        self.cached_json(&path, Stage::NlsFootprints, "NLS footprints", || {
            nls::footprints(&self.http, municipality, bbox, &self.reporter)
        })
    }

    pub fn parcels(&self, municipality: &Municipality, bbox: &BBox) -> Result<Value, IngestError> {
        let path = self.cache.municipality_file(municipality, "parcels.json");
        self.cached_json(&path, Stage::Parcels, "land parcels", || {
            parcels::parcels(&self.http, municipality, bbox, &self.reporter)
        })
    }

    pub fn osm_buildings(&self, municipality: &Municipality, official: &Value) -> Result<Value, IngestError> {
        let path = self.cache.municipality_file(municipality, "osm_buildings.json");
        let extract = osm_pbf::ensure_extract(&self.download_http, &self.cache, self.cache_max_age, &self.reporter)?;

        if self.cache.is_newer(&path, &extract)? && self.cache.is_fresh(&path, self.cache_max_age)? {
            match self.cache.load_json(&path) {
                Ok(Some(value)) => {
                    self.reporter
                        .report(Stage::OsmBuildings, "Using cached OSM buildings", Some(100));
                    return Ok(value);
                }
                Ok(None) => {}
                Err(error) => {
                    self.reporter.report(
                        Stage::OsmBuildings,
                        format!("Ignoring invalid OSM buildings cache: {error}"),
                        None,
                    );
                }
            }
        }

        let value = osm_pbf::buildings(&extract, municipality, official, &self.reporter)?;
        self.cache.save_json(&path, &value)?;
        self.reporter.report(
            Stage::OsmBuildings,
            format!("Saved OSM buildings to {}", path.display()),
            Some(100),
        );

        Ok(value)
    }

    pub fn fetch_all(&self, municipality: &Municipality) -> Result<IngestData, IngestError> {
        let official = self.ryhti_buildings(municipality)?;
        let addresses = self.ryhti_addresses(municipality)?;

        let bbox = bbox_of_points(&official)?;

        let nls = self.nls_footprints(municipality, &bbox)?;
        let parcels = self.parcels(municipality, &bbox)?;
        let osm = self.osm_buildings(municipality, &official)?;

        Ok(IngestData {
            official,
            addresses,
            nls,
            parcels,
            osm,
            bbox,
        })
    }

    fn cached_json<F: FnOnce() -> Result<Value, IngestError>>(
        &self,
        path: &Path,
        stage: Stage,
        label: &str,
        fetch: F,
    ) -> Result<Value, IngestError> {
        if self.cache.is_fresh(path, self.cache_max_age)? {
            match self.cache.load_json(path) {
                Ok(Some(value)) => {
                    self.reporter.report(stage, format!("Using cached {label}"), Some(100));
                    return Ok(value);
                }
                Ok(None) => {}
                Err(error) => {
                    self.reporter
                        .report(stage, format!("Ignoring invalid {label} cache: {error}"), None);
                }
            }
        }

        let value = fetch()?;
        self.cache.save_json(path, &value)?;
        self.reporter
            .report(stage, format!("Saved {label} to {}", path.display()), Some(100));

        Ok(value)
    }
}
