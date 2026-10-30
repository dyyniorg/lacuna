//! Progress reporting for the ingest pipeline.

use std::sync::Arc;

/// One stage of the ingest pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    RyhtiBuildings,
    RyhtiAddresses,
    NlsFootprints,
    Parcels,
    OsmExtract,
    OsmBuildings,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Self::RyhtiBuildings => "Ryhti buildings",
            Self::RyhtiAddresses => "Ryhti addresses",
            Self::NlsFootprints => "NLS footprints",
            Self::Parcels => "Land parcels",
            Self::OsmExtract => "Finland OSM extract",
            Self::OsmBuildings => "OSM buildings",
        }
    }

    /// Name of the cached artifact for this stage, when it is municipality-specific.
    pub fn file_name(self) -> Option<&'static str> {
        match self {
            Self::OsmExtract => None,
            Self::RyhtiBuildings => Some("ryhti_buildings.json"),
            Self::RyhtiAddresses => Some("ryhti_addresses.json"),
            Self::NlsFootprints => Some("nls_footprints.json"),
            Self::Parcels => Some("parcels.json"),
            Self::OsmBuildings => Some("osm_buildings.json"),
        }
    }

    /// Returns true when the stage has a known maximum, so a percentage is meaningful.
    pub fn has_known_total(self) -> bool {
        self == Self::OsmExtract
    }
}

/// A progress update emitted by the ingest client.
#[derive(Clone, Debug)]
pub struct Progress {
    pub stage: Stage,
    pub message: String,
    pub percent: Option<u8>,
}

type Sink = Arc<dyn Fn(Progress) + Send + Sync>;

/// Forwards progress updates to a caller-provided sink.
#[derive(Clone, Default)]
pub struct Reporter {
    sink: Option<Sink>,
}

impl Reporter {
    pub fn new<F: Fn(Progress) + Send + Sync + 'static>(sink: F) -> Self {
        Self {
            sink: Some(Arc::new(sink)),
        }
    }

    pub fn report(&self, stage: Stage, message: impl Into<String>) {
        self.send(stage, message.into(), None);
    }

    /// Sends an update. Percentages are only forwarded for stages with a known total.
    pub fn report_percent(&self, stage: Stage, message: impl Into<String>, percent: Option<u8>) {
        let percent = percent.filter(|_| stage.has_known_total());
        self.send(stage, message.into(), percent);
    }

    fn send(&self, stage: Stage, message: String, percent: Option<u8>) {
        if let Some(sink) = &self.sink {
            sink(Progress {
                stage,
                message,
                percent,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[test]
    fn only_the_extract_reports_percentages() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let reporter = Reporter::new(move |progress| sink.lock().unwrap().push(progress));

        reporter.report(Stage::RyhtiBuildings, "page");
        reporter.report_percent(Stage::OsmExtract, "download", Some(50));
        reporter.report_percent(Stage::RyhtiBuildings, "ignored", Some(50));

        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].percent, None);
        assert_eq!(seen[1].percent, Some(50));
        assert_eq!(seen[2].percent, None);
    }
}
