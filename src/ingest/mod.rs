//! Input data ingestion for the local review tool.
//!
//! The client fetches Ryhti buildings and addresses, NLS footprints, land
//! parcels, and OSM buildings from a fresh Finland PBF extract. Responses are
//! cached under the configured cache directory.

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

pub use cache::Cache;
pub use error::IngestError;
pub use geo::{BBox, bbox_of_points, outer_rings, point_of_coordinates, point_of_feature};
pub(crate) use http::user_agent;
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
    download_http: reqwest::blocking::Client, // for the large OSM PBF download
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

    pub fn ryhti_buildings(&self, municipality: &Municipality) -> Result<Value, IngestError> {
        self.cached_json(municipality, Stage::RyhtiBuildings, "Ryhti buildings", None, || {
            ryhti::buildings(&self.http, municipality, &self.reporter)
        })
    }

    pub fn ryhti_addresses(&self, municipality: &Municipality) -> Result<Value, IngestError> {
        self.cached_json(municipality, Stage::RyhtiAddresses, "Ryhti addresses", None, || {
            ryhti::addresses(&self.http, municipality, &self.reporter)
        })
    }

    pub fn nls_footprints(&self, municipality: &Municipality, bbox: &BBox) -> Result<Value, IngestError> {
        self.cached_json(municipality, Stage::NlsFootprints, "NLS footprints", None, || {
            nls::footprints(&self.http, municipality, bbox, &self.reporter)
        })
    }

    pub fn parcels(&self, municipality: &Municipality, bbox: &BBox) -> Result<Value, IngestError> {
        self.cached_json(municipality, Stage::Parcels, "land parcels", None, || {
            parcels::parcels(&self.http, municipality, bbox, &self.reporter)
        })
    }

    pub fn osm_buildings(&self, municipality: &Municipality, bbox: &BBox) -> Result<Value, IngestError> {
        let extract = osm_pbf::ensure_extract(&self.download_http, &self.cache, self.cache_max_age, &self.reporter)?;

        self.cached_json(
            municipality,
            Stage::OsmBuildings,
            "OSM buildings",
            Some(&extract),
            || osm_pbf::buildings(&extract, municipality, bbox, &self.reporter),
        )
    }

    pub fn fetch_all(&self, municipality: &Municipality) -> Result<IngestData, IngestError> {
        let official = self.ryhti_buildings(municipality)?;
        let addresses = self.ryhti_addresses(municipality)?;

        let bbox = bbox_of_points(&official)?;

        let nls = self.nls_footprints(municipality, &bbox)?;
        let parcels = self.parcels(municipality, &bbox)?;
        let osm = self.osm_buildings(municipality, &bbox)?;

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
        municipality: &Municipality,
        stage: Stage,
        label: &str,
        newer_than: Option<&Path>,
        fetch: F,
    ) -> Result<Value, IngestError> {
        let path = self.cache.path(municipality, stage);
        let newer = match newer_than {
            Some(than) => self.cache.is_newer(&path, than)?,
            None => true,
        };

        // NOTE: osm_buildings.json is a derivation of the og. PBF, thus
        // the PBF extract might be replaced in some cases even if the .json
        // is younger than our max. limit -> 2 "separate" checks to ensure freshness

        if newer && self.cache.is_fresh(&path, self.cache_max_age)? {
            match self.cache.load_json(&path) {
                Ok(Some(value)) => {
                    self.reporter.report(stage, format!("Using cached {label}"));
                    return Ok(value);
                }
                Ok(None) => {}
                Err(error) => {
                    self.reporter
                        .report(stage, format!("Ignoring invalid {label} cache: {error}"));
                }
            }
        }

        let value = fetch()?;
        self.cache.save_json(&path, &value)?;
        self.reporter
            .report(stage, format!("Saved {label} to {}", path.display()));

        Ok(value)
    }
}
