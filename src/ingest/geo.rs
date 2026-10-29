//! Geographic helpers for official points and WFS requests.

use proj4rs::proj::Proj;
use proj4rs::transform::transform;
use serde_json::Value;

use super::error::IngestError;

const BBOX_MARGIN: f64 = 0.02;

/// A geographic bounding box in WGS84 degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BBox {
    pub min_lon: f64,
    pub min_lat: f64,
    pub max_lon: f64,
    pub max_lat: f64,
}

impl BBox {
    pub fn width(&self) -> f64 {
        self.max_lon - self.min_lon
    }

    pub fn height(&self) -> f64 {
        self.max_lat - self.min_lat
    }
}

/// Returns the bounding box around all official points, with a safety margin.
pub fn bbox_of_points(official: &Value) -> Result<BBox, IngestError> {
    let mut min_lon = f64::INFINITY;
    let mut min_lat = f64::INFINITY;
    let mut max_lon = f64::NEG_INFINITY;
    let mut max_lat = f64::NEG_INFINITY;

    for feature in official.get("features").and_then(Value::as_array).into_iter().flatten() {
        let Some(coordinates) = feature
            .get("geometry")
            .and_then(|geometry| geometry.get("coordinates"))
            .and_then(Value::as_array)
        else {
            continue;
        };

        if coordinates.len() < 2 {
            continue;
        }

        let (Some(lon), Some(lat)) = (coordinates[0].as_f64(), coordinates[1].as_f64()) else {
            continue;
        };

        min_lon = min_lon.min(lon);
        min_lat = min_lat.min(lat);
        max_lon = max_lon.max(lon);
        max_lat = max_lat.max(lat);
    }

    if min_lon > max_lon {
        return Err(IngestError::NoOfficialPoints);
    }

    Ok(BBox {
        min_lon: min_lon - BBOX_MARGIN,
        min_lat: min_lat - BBOX_MARGIN,
        max_lon: max_lon + BBOX_MARGIN,
        max_lat: max_lat + BBOX_MARGIN,
    })
}

/// Projects a WGS84 bounding box to ETRS-TM35FIN meters.
pub fn bbox_in_3067(bbox: &BBox) -> Result<(f64, f64, f64, f64), IngestError> {
    let from = Proj::from_proj_string("+proj=longlat +ellps=WGS84 +datum=WGS84 +no_defs")
        .map_err(|error| IngestError::InvalidData(format!("invalid source projection: {error}")))?;
    let to = Proj::from_proj_string("+proj=utm +zone=35 +ellps=GRS80 +units=m +no_defs")
        .map_err(|error| IngestError::InvalidData(format!("invalid target projection: {error}")))?;

    let mut south_west = (bbox.min_lon.to_radians(), bbox.min_lat.to_radians(), 0.0);
    let mut north_east = (bbox.max_lon.to_radians(), bbox.max_lat.to_radians(), 0.0);

    transform(&from, &to, &mut south_west)
        .map_err(|error| IngestError::InvalidData(format!("failed to project bbox: {error}")))?;
    transform(&from, &to, &mut north_east)
        .map_err(|error| IngestError::InvalidData(format!("failed to project bbox: {error}")))?;

    Ok((south_west.0, south_west.1, north_east.0, north_east.1))
}
