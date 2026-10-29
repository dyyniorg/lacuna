//! Finland OSM PBF extract handling.

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use osmpbf::{Element, ElementReader};
use reqwest::blocking::Client;
use serde_json::{Value, json};

use crate::config::Municipality;

use super::cache::Cache;
use super::error::IngestError;
use super::geo::{self, BBox};
use super::http;
use super::report::{Reporter, Stage};

const GEOFABRIK_FINLAND: &str = "https://download.geofabrik.de/europe/finland-latest.osm.pbf";

/// Returns a fresh Finland extract, downloading a replacement when needed.
pub fn ensure_extract(
    client: &Client,
    cache: &Cache,
    max_age: Duration,
    reporter: &Reporter,
) -> Result<PathBuf, IngestError> {
    let path = cache.pbf_path();

    match fs::metadata(&path) {
        Ok(metadata) => {
            let age = metadata
                .modified()
                .ok()
                .and_then(|modified| SystemTime::now().duration_since(modified).ok());

            if let Some(age) = age {
                if age < max_age {
                    reporter.report(
                        Stage::OsmExtract,
                        format!("Using cached Finland extract ({} days old)", age.as_secs() / 86_400),
                        Some(100),
                    );
                    return Ok(path);
                }
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    reporter.report(Stage::OsmExtract, "Downloading the Finland OSM extract", Some(0));

    http::download_file(client, GEOFABRIK_FINLAND, &path, |done, total| {
        let percent = total
            .filter(|total| *total > 0)
            .map(|total| ((done * 100) / total).min(100) as u8);
        reporter.report(Stage::OsmExtract, format!("Downloaded {} MiB", done >> 20), percent);
    })?;

    reporter.report(Stage::OsmExtract, "Finland OSM extract is ready", Some(100));

    Ok(path)
}

/// Extracts building ways inside the municipality bounding box from a PBF file.
pub fn buildings(
    extract: &Path,
    municipality: &Municipality,
    official: &Value,
    reporter: &Reporter,
) -> Result<Value, IngestError> {
    let bbox = geo::bbox_of_points(official)?;

    reporter.report(
        Stage::OsmBuildings,
        "Resolving OSM nodes inside the municipality bbox",
        None,
    );

    let nodes = nodes_in_bbox(extract, &bbox)?;

    reporter.report(Stage::OsmBuildings, format!("Resolved {} OSM nodes", nodes.len()), None);
    reporter.report(Stage::OsmBuildings, "Extracting OSM building ways", None);

    let features = building_ways(extract, &nodes)?;

    reporter.report(
        Stage::OsmBuildings,
        format!("Extracted {} OSM building ways", features.len()),
        Some(100),
    );

    Ok(json!({
        "type": "FeatureCollection",
        "name": format!("{}_osm", municipality.slug()),
        "source": "Geofabrik Finland extract",
        "features": features,
    }))
}

fn nodes_in_bbox(path: &Path, bbox: &BBox) -> Result<HashMap<i64, (f64, f64)>, IngestError> {
    let bbox = *bbox;
    let reader = ElementReader::from_path(path)
        .map_err(|error| IngestError::InvalidData(format!("failed to open OSM extract: {error}")))?;

    reader
        .par_map_reduce(
            move |element| {
                let mut map: HashMap<i64, (f64, f64)> = HashMap::new();
                let (id, lon, lat) = match element {
                    Element::Node(node) => (node.id(), node.lon(), node.lat()),
                    Element::DenseNode(node) => (node.id(), node.lon(), node.lat()),
                    _ => return map,
                };

                if lon >= bbox.min_lon && lon <= bbox.max_lon && lat >= bbox.min_lat && lat <= bbox.max_lat {
                    map.insert(id, (lon, lat));
                }

                map
            },
            HashMap::new,
            |mut left, right| {
                left.extend(right);
                left
            },
        )
        .map_err(|error| IngestError::InvalidData(format!("failed to read OSM extract: {error}")))
}

fn building_ways(path: &Path, nodes: &HashMap<i64, (f64, f64)>) -> Result<Vec<Value>, IngestError> {
    let reader = ElementReader::from_path(path)
        .map_err(|error| IngestError::InvalidData(format!("failed to open OSM extract: {error}")))?;

    reader
        .par_map_reduce(
            move |element| {
                let mut out: Vec<Value> = Vec::new();
                let Element::Way(way) = element else {
                    return out;
                };
                let mut tags = serde_json::Map::new();
                let mut building = false;

                for (key, value) in way.tags() {
                    tags.insert(key.to_string(), json!(value));

                    if key == "building" {
                        building = true;
                    }
                }

                if !building {
                    return out;
                }

                let mut ring: Vec<(f64, f64)> = Vec::new();

                for node_id in way.refs() {
                    // NOTE: a missing node means the way reaches outside the bounding box
                    let Some(location) = nodes.get(&node_id) else {
                        return out;
                    };
                    ring.push(*location);
                }

                if ring.len() < 4 || ring.first() != ring.last() {
                    return out;
                }

                let coordinates = ring.iter().map(|(lon, lat)| json!([lon, lat])).collect::<Vec<Value>>();

                out.push(json!({
                    "type": "Feature",
                    "id": format!("way/{}", way.id()),
                    "geometry": {"type": "Polygon", "coordinates": vec![coordinates]},
                    "properties": tags,
                }));

                out
            },
            Vec::new,
            |mut left, mut right| {
                left.append(&mut right);
                left
            },
        )
        .map_err(|error| IngestError::InvalidData(format!("failed to read OSM extract: {error}")))
}
