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

    pub fn report(&self, stage: Stage, message: impl Into<String>, percent: Option<u8>) {
        if let Some(sink) = &self.sink {
            sink(Progress {
                stage,
                message: message.into(),
                percent,
            });
        }
    }
}
