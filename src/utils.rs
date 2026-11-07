//! Command-line parameters and small shared helpers.
//!
//! The primary interface is the local web UI, but defaults can be overwritten
//! via the CLI during tool startup.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

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
    /// OSM username used to look up the latest changeset for statistics.
    osm_user: Option<String>,
    /// Statistics backend URL.
    stats_url: Option<String>,
    /// Statistics backend bearer token.
    stats_token: Option<String>,
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
            osm_user: None,
            stats_url: None,
            stats_token: None,
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

    pub fn stats(&self) -> Option<(&str, &str, &str)> {
        Some((
            self.osm_user.as_deref()?,
            self.stats_url.as_deref()?,
            self.stats_token.as_deref()?,
        ))
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
            "--osm-user" => {
                let value = args
                    .next()
                    .ok_or_else(|| InvalidArgError::new("The --osm-user option requires a username"))?;
                params.osm_user = Some(value.clone());
            }
            "--stats-url" => {
                let value = args
                    .next()
                    .ok_or_else(|| InvalidArgError::new("The --stats-url option requires a URL"))?;
                params.stats_url = Some(value.clone());
            }
            "--stats-token" => {
                let value = args
                    .next()
                    .ok_or_else(|| InvalidArgError::new("The --stats-token option requires a token"))?;
                params.stats_token = Some(value.clone());
            }
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

    let stats = [&params.osm_user, &params.stats_url, &params.stats_token]
        .into_iter()
        .filter(|value| value.is_some())
        .count();

    if stats != 0 && stats != 3 {
        return Err(InvalidArgError::new(
            "Use --osm-user, --stats-url, and --stats-token together",
        ));
    }

    Ok(params)
}

/// Writes JSON atomically through a temporary file.
pub fn save_json<T: serde::Serialize>(path: &Path, value: &T, pretty: bool) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let text = if pretty {
        serde_json::to_string_pretty(value)
    } else {
        serde_json::to_string(value)
    }
    .map_err(std::io::Error::other)?;

    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, path)?;

    Ok(())
}

/// Returns the feature array of a GeoJSON feature collection.
pub fn features(value: &Value) -> &[Value] {
    value
        .get("features")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// Returns a unique id for a review round.
pub fn now_id() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn stats_flags_must_be_complete() {
        assert!(parse_args(&args(&["--osm-user", "tester"])).is_err());
        assert!(parse_args(&args(&["--stats-url", "http://localhost/stats"])).is_err());
        assert!(parse_args(&args(&["--stats-token", "token"])).is_err());
        assert!(
            parse_args(&args(&[
                "--osm-user",
                "tester",
                "--stats-url",
                "http://localhost/stats",
                "--stats-token",
                "token",
            ]))
            .is_ok()
        );
    }
}

#[cfg(test)]
pub mod test_support {
    use std::ops::Deref;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Temporary directory that removes itself on drop.
    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new(prefix: &str, label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "lacuna-{prefix}-test-{label}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));

            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("failed to create test directory");

            Self(path)
        }
    }

    impl Deref for TempDir {
        type Target = Path;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
