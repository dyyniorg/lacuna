//! Error types for the ingest client.

use std::fmt;
use std::path::PathBuf;

/// Errors produced while fetching or caching input data.
#[derive(Debug)]
pub enum IngestError {
    Http(reqwest::Error),
    HttpStatus { status: reqwest::StatusCode, url: String },
    RetriesExhausted(String),
    Io(std::io::Error),
    Json(serde_json::Error),
    InvalidCache { path: PathBuf, message: String },
    InvalidData(String),
    NoOfficialPoints,
}

impl fmt::Display for IngestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(error) => write!(f, "HTTP request failed: {error}"),
            Self::HttpStatus { status, url } => write!(f, "HTTP {status} for {url}"),
            Self::RetriesExhausted(message) => write!(f, "Request failed after retries: {message}"),
            Self::Io(error) => write!(f, "File operation failed: {error}"),
            Self::Json(error) => write!(f, "JSON error: {error}"),
            Self::InvalidCache { path, message } => {
                write!(f, "Invalid cache file {}: {message}", path.display())
            }
            Self::InvalidData(message) => write!(f, "Invalid data: {message}"),
            Self::NoOfficialPoints => write!(f, "No official points found"),
        }
    }
}

impl std::error::Error for IngestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Http(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for IngestError {
    fn from(error: reqwest::Error) -> Self {
        Self::Http(error)
    }
}

impl From<std::io::Error> for IngestError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for IngestError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}
