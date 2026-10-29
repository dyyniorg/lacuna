//! Command-line parameters and small shared helpers.
//!
//! The primary interface is the local web UI, but defaults can be overwritten
//! via the CLI during tool startup.

use std::path::{Path, PathBuf};
use std::time::Duration;

pub const DEFAULT_CACHE_DIR: &str = "data";
pub const DEFAULT_CACHE_MAX_AGE_DAYS: u64 = 7;
pub const DEFAULT_LISTEN: &str = "127.0.0.1:3000";

const ALL_INTERFACES_LISTEN: &str = "0.0.0.0:3000";

/// Runtime configuration for the application.
#[derive(Clone, Debug)]
pub struct Parameters {
    /// Root directory for cached input files. Defaults to `data`.
    cache_dir: PathBuf,
    /// Maximum age of a cached resource before it is fetched again. Defaults
    /// to 7 days.
    cache_max_age: Duration,
    /// Listen on all network interfaces instead of localhost.
    listen_all: bool,
    /// Run the full ingest and review pipeline.
    review: bool,
    /// Recompute addresses for one reviewed grid cell.
    adjust: Option<String>,
    /// Allow replacing existing review output.
    overwrite: bool,
    /// Run ingest only and print the result counts.
    fetch: bool,
    /// Municipality code selected for this run. `None` prevents a CLI run and
    /// lets the web UI choose the municipality.
    municipality: Option<String>,
}

impl Default for Parameters {
    fn default() -> Self {
        Self {
            cache_dir: PathBuf::from(DEFAULT_CACHE_DIR),
            cache_max_age: Duration::from_secs(DEFAULT_CACHE_MAX_AGE_DAYS * 86_400),
            listen_all: false,
            review: false,
            adjust: None,
            overwrite: false,
            fetch: false,
            municipality: None,
        }
    }
}

impl Parameters {
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    pub fn cache_max_age(&self) -> Duration {
        self.cache_max_age
    }

    pub fn listen(&self) -> &'static str {
        if self.listen_all {
            ALL_INTERFACES_LISTEN
        } else {
            DEFAULT_LISTEN
        }
    }

    pub fn review(&self) -> bool {
        self.review
    }

    pub fn adjust(&self) -> Option<&str> {
        self.adjust.as_deref()
    }

    pub fn overwrite(&self) -> bool {
        self.overwrite
    }

    pub fn fetch(&self) -> bool {
        self.fetch
    }

    pub fn municipality(&self) -> Option<&str> {
        self.municipality.as_deref()
    }
}

#[derive(Debug)]
pub struct InvalidArgError(String);

impl InvalidArgError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for InvalidArgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for InvalidArgError {}

pub fn parse_args(raw_args: &[String]) -> Result<Parameters, InvalidArgError> {
    let mut params = Parameters::default();
    let mut args = raw_args.iter();

    while let Some(arg) = args.next() {
        match arg.to_lowercase().as_str() {
            "--cache" => {
                let value = args
                    .next()
                    .ok_or_else(|| InvalidArgError::new("The --cache option requires a directory"))?;
                params.cache_dir = PathBuf::from(value);
            }
            "--cache-max-age" => {
                let value = args
                    .next()
                    .ok_or_else(|| InvalidArgError::new("The --cache-max-age option requires a number of days"))?;
                let days = value
                    .parse::<u64>()
                    .map_err(|_| InvalidArgError::new("The --cache-max-age option requires a number of days"))?;
                params.cache_max_age = Duration::from_secs(days.saturating_mul(86_400));
            }
            "--listen-all" => params.listen_all = true,
            "--municipality" => {
                let value = args
                    .next()
                    .ok_or_else(|| InvalidArgError::new("The --municipality option requires a code"))?;
                params.municipality = Some(value.clone());
            }
            "--review" => params.review = true,
            "--adjust" => {
                let value = args
                    .next()
                    .ok_or_else(|| InvalidArgError::new("The --adjust option requires a cell"))?;
                params.adjust = Some(value.clone());
            }
            "--overwrite" => params.overwrite = true,
            "--fetch" => params.fetch = true,
            other => {
                return Err(InvalidArgError::new(format!("Unknown argument: {other}")));
            }
        }
    }

    let modes = [params.fetch, params.review, params.adjust.is_some()]
        .into_iter()
        .filter(|enabled| *enabled)
        .count();

    if modes > 1 {
        return Err(InvalidArgError::new("Use only one of --fetch, --review, or --adjust"));
    }

    Ok(params)
}
