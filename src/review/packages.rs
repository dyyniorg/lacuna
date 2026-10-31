//! Grid cells and per-cell review packages.

use std::path::Path;

use serde_json::Value;

use crate::ingest::{BBox, point_of_coordinates, point_of_feature};
use crate::utils;

use super::ReviewError;
use super::diff::AddressFix;
use super::osm_io::{self, ExistingWay};

const CELL_LON: f64 = 0.04;
const CELL_LAT: f64 = 0.02;
const CELL_MARGIN: f64 = 0.005;

/// One populated grid cell.
#[derive(Clone, Debug)]
pub struct Cell {
    pub number: String,
    pub base_bbox: BBox,
    pub cut_bbox: BBox,
}

/// Summary for one written review package.
#[derive(Clone, Debug)]
pub struct CellPackage {
    pub number: String,
    pub buildings: usize,
    pub addresses: usize,
    pub missing_refs: Vec<String>,
}

pub fn cells_of(official: &Value, bbox: &BBox) -> Vec<Cell> {
    let mut used: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();

    for feature in utils::features(official) {
        let Some((lon, lat)) = point_of_feature(feature) else {
            continue;
        };

        if lon < bbox.min_lon || lon > bbox.max_lon || lat < bbox.min_lat || lat > bbox.max_lat {
            continue;
        }

        let col = ((lon - bbox.min_lon) / CELL_LON) as usize;
        let row = ((lat - bbox.min_lat) / CELL_LAT) as usize;

        used.insert((row, col));
    }

    let mut cells: Vec<Cell> = used
        .into_iter()
        .map(|(row, col)| {
            let base_bbox = BBox {
                min_lon: bbox.min_lon + col as f64 * CELL_LON,
                min_lat: bbox.min_lat + row as f64 * CELL_LAT,
                max_lon: bbox.min_lon + (col + 1) as f64 * CELL_LON,
                max_lat: bbox.min_lat + (row + 1) as f64 * CELL_LAT,
            };

            Cell {
                number: format!("{row:02}_{col:02}"),
                base_bbox,
                cut_bbox: BBox {
                    min_lon: base_bbox.min_lon - CELL_MARGIN,
                    min_lat: base_bbox.min_lat - CELL_MARGIN,
                    max_lon: base_bbox.max_lon + CELL_MARGIN,
                    max_lat: base_bbox.max_lat + CELL_MARGIN,
                },
            }
        })
        .collect();

    cells.sort_by(|left, right| left.number.cmp(&right.number));

    cells
}

pub fn write_packages(
    review_dir: &Path,
    official: &Value,
    result: &Value,
    address_missing: &[Value],
    bbox: &BBox,
) -> Result<Vec<CellPackage>, ReviewError> {
    let cells = cells_of(official, bbox);

    let mut packages = Vec::new();

    for cell in &cells {
        let missing: Vec<Value> = utils::features(result)
            .iter()
            .filter(|feature| {
                feature["properties"]["match_state"] == "missing"
                    && property_in_bbox(feature, "official_point", &cell.base_bbox)
            })
            .cloned()
            .collect();

        let addresses_here: Vec<Value> = address_missing
            .iter()
            .filter(|feature| {
                let center_inside = feature_center(feature).is_some_and(|center| inside_bbox(center, &cell.base_bbox));
                center_inside || property_in_bbox(feature, "owner_point", &cell.base_bbox)
            })
            .cloned()
            .collect();

        if missing.is_empty() && addresses_here.is_empty() {
            continue;
        }

        let dir = review_dir.join(&cell.number);

        osm_io::ensure_dir(&dir)?;

        let bbox_text = format!(
            "bbox: {:.6},{:.6},{:.6},{:.6}\n\
             https://www.openstreetmap.org/#map=15/{:.5}/{:.5}\n",
            cell.cut_bbox.min_lat,
            cell.cut_bbox.min_lon,
            cell.cut_bbox.max_lat,
            cell.cut_bbox.max_lon,
            (cell.cut_bbox.min_lat + cell.cut_bbox.max_lat) / 2.0,
            (cell.cut_bbox.min_lon + cell.cut_bbox.max_lon) / 2.0
        );
        std::fs::write(dir.join("osm_bbox.txt"), bbox_text)?;

        if !missing.is_empty() {
            osm_io::save_osm(&dir.join("buildings.osm"), &missing)?;
        }

        let address_ways: Vec<ExistingWay> = addresses_here.iter().filter_map(existing_way_from_feature).collect();

        osm_io::save_existing_osm(&dir.join("addresses.osm"), &address_ways)?;

        let mut missing_refs: Vec<String> = missing
            .iter()
            .filter_map(|feature| {
                feature["properties"]["permanent_building_identifier"]
                    .as_str()
                    .map(str::to_string)
            })
            .collect();
        missing_refs.sort();
        missing_refs.dedup();

        packages.push(CellPackage {
            number: cell.number.clone(),
            buildings: missing.len(),
            addresses: addresses_here.len(),
            missing_refs,
        });
    }

    Ok(packages)
}

pub fn feature_center(feature: &Value) -> Option<(f64, f64)> {
    let geometry = feature.get("geometry")?;
    let ring = match geometry.get("type").and_then(Value::as_str)? {
        "Polygon" => geometry.get("coordinates")?.as_array()?.first()?.as_array()?,
        "MultiPolygon" => geometry
            .get("coordinates")?
            .as_array()?
            .first()?
            .as_array()?
            .first()?
            .as_array()?,
        _ => return None,
    };

    let mut lon = 0.0;
    let mut lat = 0.0;
    let mut count = 0.0;
    for vertex in ring {
        let vertex = vertex.as_array()?;
        if vertex.len() < 2 {
            continue;
        }
        lon += vertex[0].as_f64()?;
        lat += vertex[1].as_f64()?;
        count += 1.0;
    }

    if count == 0.0 {
        return None;
    }

    Some((lon / count, lat / count))
}

pub fn inside_bbox(point: (f64, f64), bbox: &BBox) -> bool {
    point.0 >= bbox.min_lon && point.0 < bbox.max_lon && point.1 >= bbox.min_lat && point.1 < bbox.max_lat
}

fn property_in_bbox(feature: &Value, key: &str, bbox: &BBox) -> bool {
    let Some(point) = feature.get("properties").and_then(|properties| properties.get(key)) else {
        return false;
    };
    let Some(array) = point.as_array() else {
        return false;
    };

    array.len() == 2 && point_of_coordinates(point).is_some_and(|point| inside_bbox(point, bbox))
}

fn text_value<'a>(address: Option<&'a Value>, key: &str) -> &'a str {
    address
        .and_then(|address| address.get(key))
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn address_tags(entries: [(&str, &str); 4]) -> Vec<(String, String)> {
    let mut tags = vec![("lacuna:review".to_string(), "yes".to_string())];

    for (key, value) in entries {
        if !value.is_empty() {
            tags.push((key.to_string(), value.to_string()));
        }
    }

    tags
}

pub fn existing_way_from_feature(feature: &Value) -> Option<ExistingWay> {
    let address = feature.get("properties")?.get("official_address");
    let tags = address_tags([
        ("addr:housenumber", text_value(address, "housenumber")),
        ("addr:street", text_value(address, "street")),
        ("addr:postcode", text_value(address, "postcode")),
        ("addr:city", text_value(address, "city")),
    ]);

    existing_way_with_tags(feature, tags)
}

pub fn existing_way_from_fix(feature: &Value, fix: &AddressFix) -> Option<ExistingWay> {
    let tags = address_tags([
        ("addr:housenumber", &fix.housenumber),
        ("addr:street", &fix.street),
        ("addr:postcode", &fix.postcode),
        ("addr:city", &fix.city),
    ]);

    existing_way_with_tags(feature, tags)
}

fn existing_way_with_tags(feature: &Value, tags: Vec<(String, String)>) -> Option<ExistingWay> {
    let id = feature
        .get("id")
        .and_then(Value::as_str)?
        .strip_prefix("way/")?
        .parse::<i64>()
        .ok()?;

    let geometry = feature.get("geometry")?;

    if geometry.get("type").and_then(Value::as_str) != Some("Polygon") {
        return None;
    }

    let ring = ring_of(geometry.get("coordinates")?.as_array()?.first()?)?;

    Some(ExistingWay { id, ring, tags })
}

fn ring_of(ring: &Value) -> Option<Vec<(f64, f64)>> {
    let mut vertices = Vec::new();

    for vertex in ring.as_array()? {
        let vertex = vertex.as_array()?;
        if vertex.len() < 2 {
            continue;
        }
        vertices.push((vertex[0].as_f64()?, vertex[1].as_f64()?));
    }
    Some(vertices)
}
