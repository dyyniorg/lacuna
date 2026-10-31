//! Review processing: comparison, grid packages, and address adjustment.

mod adjust;
mod diff;
mod osm_io;
mod packages;
mod state;

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::config::Municipality;
use crate::ingest::{IngestData, IngestError};
use crate::utils;

pub use adjust::adjust;
pub use diff::Counts;
pub use packages::CellPackage;
pub use state::{ReviewState, load as load_state};

/// Review processing errors.
#[derive(Debug)]
pub enum ReviewError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Ingest(String),
    ExistingOutput(Vec<PathBuf>),
    MissingInput(PathBuf),
    CellNotFound(String),
    InvalidData(String),
}

impl fmt::Display for ReviewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "File operation failed: {error}"),
            Self::Json(error) => write!(f, "JSON error: {error}"),
            Self::Ingest(message) => write!(f, "Ingest error: {message}"),
            Self::ExistingOutput(paths) => {
                write!(f, "Review output already exists: {} files", paths.len())
            }
            Self::MissingInput(path) => {
                write!(f, "Missing review input: {}", path.display())
            }
            Self::CellNotFound(cell) => write!(f, "Review cell not found: {cell}"),
            Self::InvalidData(message) => write!(f, "Invalid data: {message}"),
        }
    }
}

impl std::error::Error for ReviewError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ReviewError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for ReviewError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<IngestError> for ReviewError {
    fn from(error: IngestError) -> Self {
        Self::Ingest(error.to_string())
    }
}

/// Result of a full review run.
#[derive(Debug)]
pub struct ReviewOutput {
    pub counts: Counts,
    pub cells: Vec<CellPackage>,
    pub address_fixes: usize,
    pub round_id: String,
}

/// Returns the subdirectories of one review directory.
fn review_dirs(dir: &Path) -> Result<Vec<PathBuf>, ReviewError> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut paths = Vec::new();

    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            paths.push(path);
        }
    }

    Ok(paths)
}

/// Returns existing review output paths that require overwrite confirmation.
pub fn existing_outputs(municipality: &Municipality, cache_dir: &Path) -> Result<Vec<PathBuf>, ReviewError> {
    let dir = cache_dir.join(municipality.slug());

    let mut paths: Vec<PathBuf> = ["review_state.json", "buildings.osm", "addresses.osm"]
        .into_iter()
        .map(|name| dir.join(name))
        .filter(|path| path.exists())
        .collect();

    paths.extend(review_dirs(&dir.join("review"))?);

    Ok(paths)
}

/// Lists the grid cells that have review packages.
pub fn review_cells(municipality: &Municipality, cache_dir: &Path) -> Result<Vec<String>, ReviewError> {
    let dir = cache_dir.join(municipality.slug()).join("review");

    let mut cells: Vec<String> = review_dirs(&dir)?
        .iter()
        .filter_map(|path| path.file_name().and_then(|name| name.to_str()).map(str::to_string))
        .collect();

    cells.sort();

    Ok(cells)
}

/// Indexes GeoJSON features by their `id`.
pub(super) fn features_by_id(value: &Value) -> HashMap<&str, &Value> {
    utils::features(value)
        .iter()
        .filter_map(|feature| feature.get("id").and_then(Value::as_str).map(|id| (id, feature)))
        .collect()
}

/// Maps official building ids to their original registry points.
pub(super) fn owner_points(result: &Value) -> HashMap<&str, (f64, f64)> {
    utils::features(result)
        .iter()
        .filter_map(|feature| {
            let properties = &feature["properties"];
            let id = properties.get("permanent_building_identifier")?.as_str()?;
            let point = crate::ingest::point_of_coordinates(properties.get("official_point")?)?;
            Some((id, point))
        })
        .collect()
}

/// Runs the comparison and writes all review outputs for one municipality.
pub fn run(
    data: &IngestData,
    municipality: &Municipality,
    cache_dir: &Path,
    overwrite: bool,
) -> Result<ReviewOutput, ReviewError> {
    let existing = existing_outputs(municipality, cache_dir)?;
    if !existing.is_empty() && !overwrite {
        return Err(ReviewError::ExistingOutput(existing));
    }

    let (result, counts, address_fixes) = diff::compare(
        &data.official,
        &data.nls,
        &data.osm,
        &data.addresses,
        &data.parcels,
        &diff::Overrides::default(),
    );

    let out_dir = cache_dir.join(municipality.slug());

    osm_io::ensure_dir(&out_dir)?;

    if overwrite {
        let review_dir = out_dir.join("review");
        if review_dir.exists() {
            fs::remove_dir_all(&review_dir)?;
        }
    }

    let by_id = features_by_id(&data.osm);
    let owner_points = owner_points(&result);

    let address_missing_features: Vec<Value> = address_fixes
        .iter()
        .filter_map(|fix| {
            let mut copy = (*by_id.get(fix.way_id.as_str())?).clone();
            copy["properties"]["official_address"] = json!({
                "housenumber": fix.housenumber,
                "street": fix.street,
                "postcode": fix.postcode,
                "city": fix.city,
            });
            if let Some(owner) = fix.owner.as_ref()
                && let Some(point) = owner_points.get(owner.as_str())
            {
                copy["properties"]["owner_point"] = json!([point.0, point.1]);
            }
            Some(copy)
        })
        .collect();

    let address_ways: Vec<osm_io::ExistingWay> = address_missing_features
        .iter()
        .filter_map(packages::existing_way_from_feature)
        .collect();

    osm_io::save_existing_osm(&out_dir.join("addresses.osm"), &address_ways)?;

    let missing_features: Vec<Value> = utils::features(&result)
        .iter()
        .filter(|feature| feature["properties"]["match_state"] == "missing")
        .cloned()
        .collect();

    let buildings_path = out_dir.join("buildings.osm");

    if missing_features.is_empty() {
        match fs::remove_file(&buildings_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    } else {
        osm_io::save_osm(&buildings_path, &missing_features)?;
    }

    let cells = packages::write_packages(
        &out_dir.join("review"),
        &data.official,
        &result,
        &address_missing_features,
        &data.bbox,
    )?;

    let round_id = utils::now_id();
    let missing_by_cell = cells
        .iter()
        .filter(|cell| !cell.missing_refs.is_empty())
        .map(|cell| (cell.number.clone(), cell.missing_refs.clone()))
        .collect();
    let review_state = ReviewState {
        round_id: round_id.clone(),
        missing_by_cell,
    };

    state::save(cache_dir, municipality, &review_state)?;

    Ok(ReviewOutput {
        counts,
        cells,
        address_fixes: address_ways.len(),
        round_id,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashSet};
    use std::fs;

    use serde_json::{Value, json};

    use crate::config::Municipality;
    use crate::utils::test_support::TempDir;

    use super::diff::{self, Overrides};
    use super::packages;
    use crate::ingest;

    const MUNICIPALITY: Municipality = Municipality {
        code: "291",
        name: "Kuhmoinen",
    };

    fn feature(geometry: Value, properties: Value) -> Value {
        json!({
            "type": "Feature",
            "geometry": geometry,
            "properties": properties,
        })
    }

    fn point(lon: f64, lat: f64) -> Value {
        json!({"type": "Point", "coordinates": [lon, lat]})
    }

    fn square(min_lon: f64, min_lat: f64, max_lon: f64, max_lat: f64) -> Value {
        json!({
            "type": "Polygon",
            "coordinates": [[
                [min_lon, min_lat],
                [max_lon, min_lat],
                [max_lon, max_lat],
                [min_lon, max_lat],
                [min_lon, min_lat],
            ]],
        })
    }

    fn collection(features: Vec<Value>) -> Value {
        json!({"type": "FeatureCollection", "features": features})
    }

    fn official(id: &str, key: &str, parcel: &str, lon: f64, lat: f64) -> Value {
        feature(
            point(lon, lat),
            json!({
                "permanent_building_identifier": id,
                "building_key": key,
                "property_identifier": parcel,
                "usage_status": "01",
                "demolition_date": null,
                "main_purpose": "05",
                "floor_area": 100,
            }),
        )
    }

    fn nls(id: &str, min_lon: f64, min_lat: f64, max_lon: f64, max_lat: f64) -> Value {
        feature(square(min_lon, min_lat, max_lon, max_lat), json!({"mtk_id": id}))
    }

    fn osm_way(
        id: &str,
        min_lon: f64,
        min_lat: f64,
        max_lon: f64,
        max_lat: f64,
        number: Option<&str>,
        street: Option<&str>,
    ) -> Value {
        let mut properties = serde_json::Map::new();
        properties.insert("building".to_string(), json!("yes"));

        if let Some(number) = number {
            properties.insert("addr:housenumber".to_string(), json!(number));
        }

        if let Some(street) = street {
            properties.insert("addr:street".to_string(), json!(street));
        }

        json!({
            "type": "Feature",
            "id": id,
            "geometry": square(min_lon, min_lat, max_lon, max_lat),
            "properties": properties,
        })
    }

    fn parcel(id: &str, min_lon: f64, min_lat: f64, max_lon: f64, max_lat: f64) -> Value {
        feature(
            square(min_lon, min_lat, max_lon, max_lat),
            json!({"nationalLandReference": id}),
        )
    }

    fn address(key: &str, number: &str, street: &str, lon: f64, lat: f64) -> Value {
        feature(
            point(lon, lat),
            json!({
                "building_key": key,
                "address_fin": format!("{street} {number}"),
                "address_name_fin": street,
                "number_part_of_address_number": number,
                "postal_code": "17800",
                "postal_office_fin": "KUHMOINEN",
            }),
        )
    }

    fn result_for<'a>(result: &'a Value, id: &str) -> &'a Value {
        result["features"]
            .as_array()
            .expect("missing result features")
            .iter()
            .find(|feature| feature["properties"]["permanent_building_identifier"].as_str() == Some(id))
            .unwrap_or_else(|| panic!("missing result for {id}"))
    }

    fn compare(
        official: &Value,
        nls: &Value,
        osm: &Value,
        addresses: &Value,
        parcels: &Value,
        overrides: &Overrides,
    ) -> (Value, diff::Counts, Vec<diff::AddressFix>) {
        diff::compare(official, nls, osm, addresses, parcels, overrides)
    }

    #[test]
    fn matches_states_and_assigns_addresses() {
        let official = collection(vec![
            official("A", "B1", "P1", 25.001, 61.001),
            official("B", "B2", "P1", 25.010, 61.010),
            official("C", "B3", "P2", 25.020, 61.020),
        ]);

        let nls = collection(vec![
            nls("A1", 25.0005, 61.0005, 25.0015, 61.0015),
            nls("B1", 25.0095, 61.0095, 25.0105, 61.0105),
            nls("C1", 25.019, 61.019, 25.021, 61.021),
        ]);

        let osm = collection(vec![
            osm_way("way/100", 25.0004, 61.0004, 25.0016, 61.0016, None, None),
            osm_way("way/300", 25.0098, 61.0098, 25.0108, 61.0108, None, None),
        ]);

        let parcels = collection(vec![
            parcel("P1", 24.999, 60.999, 25.012, 61.012),
            parcel("P2", 25.018, 61.018, 25.023, 61.022),
        ]);

        let addresses = collection(vec![
            address("B1", "1", "Testikatu", 25.001, 61.001),
            address("B2", "2", "Testikatu", 25.010, 61.010),
            address("B3", "3", "Testikatu", 25.020, 61.020),
        ]);

        let (result, counts, fixes) = compare(&official, &nls, &osm, &addresses, &parcels, &Overrides::default());

        assert_eq!(counts.matched, 1);
        assert_eq!(counts.changed, 1);
        assert_eq!(counts.missing, 1);
        assert_eq!(fixes.len(), 2);
        let way_ids: HashSet<&str> = fixes.iter().map(|fix| fix.way_id.as_str()).collect();
        assert!(way_ids.contains("way/100"));
        assert!(way_ids.contains("way/300"));

        let a = result_for(&result, "A");
        assert_eq!(a["properties"]["match_state"].as_str(), Some("matched"));
        assert_eq!(a["properties"]["osm_way"].as_str(), Some("way/100"));

        let b = result_for(&result, "B");
        assert_eq!(b["properties"]["match_state"].as_str(), Some("changed"));
        assert_eq!(b["properties"]["osm_way"].as_str(), Some("way/300"));

        let c = result_for(&result, "C");
        assert_eq!(c["properties"]["match_state"].as_str(), Some("missing"));
        assert_eq!(
            c["properties"]["suggested_tags"]["addr:housenumber"].as_str(),
            Some("3")
        );
        assert_eq!(
            c["properties"]["suggested_tags"]["addr:street"].as_str(),
            Some("Testikatu")
        );
    }

    #[test]
    fn overrides_control_address_placement() {
        let official = collection(vec![official("M", "BM", "P2", 25.020, 61.020)]);

        let nls = collection(vec![nls("M1", 25.019, 61.019, 25.021, 61.021)]);

        let osm = collection(vec![osm_way("way/200", 25.022, 61.020, 25.023, 61.021, None, None)]);

        let parcels = collection(vec![parcel("P2", 25.017, 61.017, 25.024, 61.023)]);

        let addresses = collection(vec![address("BM", "2", "Testikatu", 25.020, 61.020)]);

        let (result, _, fixes) = compare(&official, &nls, &osm, &addresses, &parcels, &Overrides::default());
        assert!(fixes.is_empty());
        assert_eq!(
            result_for(&result, "M")["properties"]["suggested_tags"]["addr:housenumber"].as_str(),
            Some("2")
        );

        let existing_only = Overrides {
            deleted: HashSet::new(),
            placed: HashSet::new(),
            existing_only: true,
        };
        let (_, _, fixes) = compare(&official, &nls, &osm, &addresses, &parcels, &existing_only);
        assert_eq!(fixes.len(), 1);
        assert_eq!(fixes[0].way_id, "way/200");

        let deleted = Overrides {
            deleted: HashSet::from(["M".to_string()]),
            placed: HashSet::new(),
            existing_only: true,
        };
        let (_, _, fixes) = compare(&official, &nls, &osm, &addresses, &parcels, &deleted);
        assert_eq!(fixes.len(), 1);
        assert_eq!(fixes[0].way_id, "way/200");

        let placed = Overrides {
            deleted: HashSet::new(),
            placed: HashSet::from([diff::address_key("2", "Testikatu")]),
            existing_only: true,
        };
        let (result, _, fixes) = compare(&official, &nls, &osm, &addresses, &parcels, &placed);
        assert!(fixes.is_empty());
        assert_eq!(
            result_for(&result, "M")["properties"]["address_state"].as_str(),
            Some("matched")
        );
    }

    #[test]
    fn point_only_buildings_are_not_written() {
        let official = collection(vec![official("P", "BP", "P1", 25.001, 61.001)]);

        let addresses = collection(vec![address("BP", "9", "Testikatu", 25.001, 61.001)]);

        let (result, counts, fixes) = compare(
            &official,
            &collection(Vec::new()),
            &collection(Vec::new()),
            &addresses,
            &collection(Vec::new()),
            &Overrides::default(),
        );

        assert_eq!(counts.official_current, 1);
        assert_eq!(counts.without_footprint, 1);
        assert!(fixes.is_empty());
        assert!(result["features"].as_array().expect("missing features").is_empty());
    }

    #[test]
    fn packages_use_owner_point_and_half_open_cells() {
        let dir = TempDir::new("review", "packages");
        let review_dir = dir.join("review");
        let bbox = ingest::BBox {
            min_lon: 24.98,
            min_lat: 60.98,
            max_lon: 25.10,
            max_lat: 61.10,
        };
        let official = collection(vec![official("A", "B1", "P1", 25.001, 61.001)]);
        let cells = packages::cells_of(&official, &bbox);
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].number, "01_00");
        assert!(packages::inside_bbox((25.001, 61.001), &cells[0].base_bbox));
        assert!(!packages::inside_bbox(
            (cells[0].base_bbox.max_lon, 61.01),
            &cells[0].base_bbox
        ));

        let result = collection(vec![feature(
            square(25.0005, 61.0005, 25.0015, 61.0015),
            json!({
                "official_point": [25.001, 61.001],
                "match_state": "missing",
                "permanent_building_identifier": "A",
                "suggested_tags": {
                    "building": "yes",
                    "lacuna:ref": "A",
                    "lacuna:review": "yes",
                },
            }),
        )]);
        let address_missing = vec![feature(
            square(25.04, 61.001, 25.06, 61.002),
            json!({
                "official_address": {
                    "housenumber": "1",
                    "street": "Testikatu",
                    "postcode": "17800",
                    "city": "Kuhmoinen",
                },
                "owner_point": [25.001, 61.001],
            }),
        )]
        .into_iter()
        .map(|mut feature| {
            feature["id"] = json!("way/100");
            feature
        })
        .collect::<Vec<_>>();

        let packages = packages::write_packages(&review_dir, &official, &result, &address_missing, &bbox)
            .expect("failed to write packages");

        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].number, "01_00");
        assert_eq!(packages[0].buildings, 1);
        assert_eq!(packages[0].addresses, 1);
        assert_eq!(packages[0].missing_refs, vec!["A".to_string()]);

        let cell_dir = review_dir.join("01_00");
        for name in ["buildings.osm", "addresses.osm", "osm_bbox.txt"] {
            assert!(cell_dir.join(name).is_file(), "missing {name}");
        }
        let bbox_text = fs::read_to_string(cell_dir.join("osm_bbox.txt")).unwrap();
        assert!(bbox_text.contains("bbox: 60.995000,24.975000,61.025000,25.025000"));
        assert!(bbox_text.contains("https://www.openstreetmap.org/#map=15/"));
        assert!(!bbox_text.contains("josm --download="));

        let buildings = fs::read_to_string(cell_dir.join("buildings.osm")).unwrap();
        assert!(buildings.contains("lacuna:ref"));
        assert!(buildings.contains("lacuna:review"));

        let addresses = fs::read_to_string(cell_dir.join("addresses.osm")).unwrap();
        assert!(addresses.contains("way id=\"100\""));
        assert!(addresses.contains("lacuna:review"));
    }

    #[test]
    fn packages_skip_empty_buildings_file() {
        let dir = TempDir::new("review", "skip-buildings");
        let review_dir = dir.join("review");
        let bbox = ingest::BBox {
            min_lon: 24.98,
            min_lat: 60.98,
            max_lon: 25.10,
            max_lat: 61.10,
        };
        let official = collection(vec![official("A", "B1", "P1", 25.001, 61.001)]);
        let address_missing = vec![feature(
            square(25.04, 61.001, 25.06, 61.002),
            json!({
                "official_address": {
                    "housenumber": "1",
                    "street": "Testikatu",
                    "postcode": "17800",
                    "city": "Kuhmoinen",
                },
                "owner_point": [25.001, 61.001],
            }),
        )]
        .into_iter()
        .map(|mut feature| {
            feature["id"] = json!("way/100");
            feature
        })
        .collect::<Vec<_>>();

        let packages =
            packages::write_packages(&review_dir, &official, &collection(Vec::new()), &address_missing, &bbox)
                .expect("failed to write packages");

        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].buildings, 0);
        assert_eq!(packages[0].addresses, 1);

        let cell_dir = review_dir.join(&packages[0].number);
        assert!(!cell_dir.join("buildings.osm").exists());
        assert!(cell_dir.join("addresses.osm").is_file());
    }

    #[test]
    fn run_writes_osm_outputs_and_minimal_state() {
        let dir = TempDir::new("review", "run");
        let data_dir = dir.join(MUNICIPALITY.slug());
        fs::create_dir_all(&data_dir).unwrap();

        let official = collection(vec![official("A", "B1", "P1", 25.001, 61.001)]);
        let nls = collection(vec![nls("A1", 25.0005, 61.0005, 25.0015, 61.0015)]);
        let bbox = ingest::bbox_of_points(&official).unwrap();
        let data = ingest::IngestData {
            official,
            addresses: collection(Vec::new()),
            nls,
            parcels: collection(Vec::new()),
            osm: collection(Vec::new()),
            bbox,
        };

        let output = super::run(&data, &MUNICIPALITY, &dir, false).unwrap();

        assert!(!output.round_id.is_empty());
        assert_eq!(output.cells.len(), 1);

        for name in ["buildings.osm", "addresses.osm", "review_state.json"] {
            assert!(data_dir.join(name).is_file(), "missing {name}");
        }

        let cell_dir = data_dir.join("review").join(&output.cells[0].number);
        for name in ["buildings.osm", "addresses.osm", "osm_bbox.txt"] {
            assert!(cell_dir.join(name).is_file(), "missing {name}");
        }

        let state = super::load_state(&dir, &MUNICIPALITY)
            .unwrap()
            .expect("missing review state");
        assert_eq!(state.round_id, output.round_id);
        assert_eq!(
            state.missing_by_cell.get(&output.cells[0].number),
            Some(&vec!["A".to_string()])
        );

        let existing = super::existing_outputs(&MUNICIPALITY, &dir).unwrap();
        assert!(
            existing
                .iter()
                .any(|path| path.file_name().and_then(|name| name.to_str()) == Some("review_state.json"))
        );

        for (name, value) in [
            ("ryhti_buildings.json", &data.official),
            ("ryhti_addresses.json", &data.addresses),
            ("nls_footprints.json", &data.nls),
            ("osm_buildings.json", &data.osm),
            ("parcels.json", &data.parcels),
        ] {
            fs::write(data_dir.join(name), serde_json::to_string(value).unwrap()).unwrap();
        }

        let adjust = super::adjust(&MUNICIPALITY, &dir, &output.cells[0].number).unwrap();
        assert_eq!(adjust.kept, 1);
        assert_eq!(adjust.removed, 0);
        assert_eq!(adjust.address_fixes, 0);
    }

    #[test]
    fn adjust_moves_address_to_existing_way_and_keeps_buildings_file() {
        let dir = TempDir::new("review", "adjust");
        let data_dir = dir.join(MUNICIPALITY.slug());
        fs::create_dir_all(&data_dir).unwrap();

        let official = collection(vec![official("M", "BM", "P2", 25.020, 61.020)]);

        let nls = collection(vec![nls("M1", 25.019, 61.019, 25.021, 61.021)]);

        let osm = collection(vec![osm_way("way/200", 25.022, 61.020, 25.023, 61.021, None, None)]);

        let parcels = collection(vec![parcel("P2", 25.017, 61.017, 25.024, 61.023)]);

        let addresses = collection(vec![address("BM", "2", "Testikatu", 25.020, 61.020)]);

        for (name, value) in [
            ("ryhti_buildings.json", &official),
            ("ryhti_addresses.json", &addresses),
            ("nls_footprints.json", &nls),
            ("osm_buildings.json", &osm),
            ("parcels.json", &parcels),
        ] {
            fs::write(data_dir.join(name), serde_json::to_string(value).unwrap()).unwrap();
        }

        let bbox = ingest::bbox_of_points(&official).unwrap();
        let cells = packages::cells_of(&official, &bbox);
        let cell = cells[0].number.clone();
        let state = super::ReviewState {
            round_id: "test-round".to_string(),
            missing_by_cell: BTreeMap::from([(cell.clone(), vec!["M".to_string(), "K".to_string()])]),
        };
        super::state::save(&dir, &MUNICIPALITY, &state).unwrap();

        let cell_dir = data_dir.join("review").join(&cell);
        fs::create_dir_all(&cell_dir).unwrap();
        let buildings_path = cell_dir.join("buildings.osm");
        let kept_osm = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                        <osm version=\"0.6\" generator=\"JOSM\">\n\
                        <way id=\"-2\">\n\
                        <tag k=\"building\" v=\"yes\"/>\n\
                        <tag k=\"lacuna:ref\" v=\"K\"/>\n\
                        </way>\n\
                        </osm>\n";
        fs::write(&buildings_path, kept_osm).unwrap();

        let output = super::adjust(&MUNICIPALITY, &dir, &cell).unwrap();
        assert_eq!(output.kept, 1);
        assert_eq!(output.removed, 1);
        assert_eq!(output.placed, 0);
        assert_eq!(output.address_fixes, 1);

        let addresses_osm = fs::read_to_string(cell_dir.join("addresses.osm")).unwrap();
        assert!(addresses_osm.contains("way id=\"200\""));
        assert!(addresses_osm.contains("addr:housenumber\" v=\"2\""));
        assert_eq!(fs::read_to_string(&buildings_path).unwrap(), kept_osm);
    }

    #[test]
    fn adjust_allows_missing_buildings_file_for_skipped_cell() {
        let dir = TempDir::new("review", "adjust-skipped");
        let data_dir = dir.join(MUNICIPALITY.slug());
        fs::create_dir_all(&data_dir).unwrap();

        let official = collection(vec![official("A", "B1", "P1", 25.001, 61.001)]);
        let addresses = collection(vec![address("B1", "1", "Testikatu", 25.001, 61.001)]);
        let nls = collection(vec![nls("A1", 25.0005, 61.0005, 25.0015, 61.0015)]);
        let osm = collection(vec![osm_way("way/100", 25.0005, 61.0005, 25.0015, 61.0015, None, None)]);
        let parcels = collection(vec![parcel("P1", 24.999, 60.999, 25.012, 61.012)]);

        for (name, value) in [
            ("ryhti_buildings.json", &official),
            ("ryhti_addresses.json", &addresses),
            ("nls_footprints.json", &nls),
            ("osm_buildings.json", &osm),
            ("parcels.json", &parcels),
        ] {
            fs::write(data_dir.join(name), serde_json::to_string(value).unwrap()).unwrap();
        }

        let bbox = ingest::bbox_of_points(&official).unwrap();
        let cell = packages::cells_of(&official, &bbox)[0].number.clone();
        let state = super::ReviewState {
            round_id: "test-round".to_string(),
            missing_by_cell: BTreeMap::new(),
        };
        super::state::save(&dir, &MUNICIPALITY, &state).unwrap();

        let cell_dir = data_dir.join("review").join(&cell);
        fs::create_dir_all(&cell_dir).unwrap();

        let output = super::adjust(&MUNICIPALITY, &dir, &cell).unwrap();

        assert_eq!(output.kept, 0);
        assert_eq!(output.removed, 0);
        assert!(cell_dir.join("addresses.osm").is_file());
    }

    #[test]
    fn adjust_rejects_missing_buildings_file_when_expected() {
        let dir = TempDir::new("review", "adjust-missing");
        let data_dir = dir.join(MUNICIPALITY.slug());
        let cell = "00_00";
        let cell_dir = data_dir.join("review").join(cell);
        fs::create_dir_all(&cell_dir).unwrap();

        let state = super::ReviewState {
            round_id: "test-round".to_string(),
            missing_by_cell: BTreeMap::from([(cell.to_string(), vec!["A".to_string()])]),
        };
        super::state::save(&dir, &MUNICIPALITY, &state).unwrap();

        let result = super::adjust(&MUNICIPALITY, &dir, cell);

        assert!(matches!(result, Err(super::ReviewError::MissingInput(_))));
    }

    #[test]
    fn grid_uses_sorted_plain_cell_numbers() {
        let official = collection(vec![
            official("A", "B1", "P1", 25.001, 61.001),
            official("B", "B2", "P1", 25.050, 61.030),
        ]);
        let bbox = ingest::BBox {
            min_lon: 24.98,
            min_lat: 60.98,
            max_lon: 25.10,
            max_lat: 61.10,
        };

        let cells = packages::cells_of(&official, &bbox);
        let numbers: Vec<&str> = cells.iter().map(|cell| cell.number.as_str()).collect();
        assert_eq!(numbers, vec!["01_00", "02_01"]);
        assert!(packages::inside_bbox((25.001, 61.001), &cells[0].base_bbox));
        assert!(packages::inside_bbox((25.050, 61.030), &cells[1].base_bbox));
        assert!(!packages::inside_bbox((25.001, 61.001), &cells[1].base_bbox));
        assert!(cells[1].cut_bbox.min_lon < cells[1].base_bbox.min_lon);
        assert!(cells[1].cut_bbox.max_lat > cells[1].base_bbox.max_lat);
    }
    #[test]
    fn demolished_buildings_are_skipped() {
        let mut by_status = official("A", "B1", "P1", 25.001, 61.001);
        by_status["properties"]["usage_status"] = json!("06");
        let mut by_date = official("B", "B2", "P1", 25.010, 61.010);
        by_date["properties"]["demolition_date"] = json!("2020-01-01");
        let official = collection(vec![by_status, by_date]);

        let (result, counts, fixes) = compare(
            &official,
            &collection(Vec::new()),
            &collection(Vec::new()),
            &collection(Vec::new()),
            &collection(Vec::new()),
            &Overrides::default(),
        );

        assert_eq!(counts.official_total, 2);
        assert_eq!(counts.official_current, 0);
        assert_eq!(counts.official_skipped, 2);
        assert!(fixes.is_empty());
        assert!(result["features"].as_array().expect("missing features").is_empty());
    }

    #[test]
    fn multiple_official_addresses_are_skipped() {
        let official = collection(vec![official("A", "B1", "P1", 25.001, 61.001)]);

        let nls = collection(vec![nls("A1", 25.0005, 61.0005, 25.0015, 61.0015)]);

        let osm = collection(vec![osm_way("way/100", 25.0004, 61.0004, 25.0016, 61.0016, None, None)]);

        let parcels = collection(vec![parcel("P1", 24.999, 60.999, 25.012, 61.012)]);

        let addresses = collection(vec![
            address("B1", "1", "Testikatu", 25.001, 61.001),
            address("B1", "2", "Testikatu", 25.001, 61.001),
        ]);

        let (result, counts, fixes) = compare(&official, &nls, &osm, &addresses, &parcels, &Overrides::default());

        assert_eq!(counts.address_multiple, 1);
        assert!(fixes.is_empty());
        assert_eq!(
            result_for(&result, "A")["properties"]["address_state"].as_str(),
            Some("multiple")
        );
    }

    #[test]
    fn shared_address_attaches_only_once() {
        let official = collection(vec![
            official("A", "B1", "P1", 25.001, 61.001),
            official("B", "B2", "P1", 25.010, 61.010),
        ]);

        let nls = collection(vec![
            nls("A1", 25.0005, 61.0005, 25.0015, 61.0015),
            nls("B1", 25.0095, 61.0095, 25.0105, 61.0105),
        ]);

        let osm = collection(vec![
            osm_way("way/100", 25.0004, 61.0004, 25.0016, 61.0016, None, None),
            osm_way("way/200", 25.0096, 61.0096, 25.0104, 61.0104, None, None),
        ]);

        let parcels = collection(vec![parcel("P1", 24.999, 60.999, 25.012, 61.012)]);

        let addresses = collection(vec![
            address("B1", "1", "Testikatu", 25.001, 61.001),
            address("B2", "1", "Testikatu", 25.010, 61.010),
        ]);

        let (result, counts, fixes) = compare(&official, &nls, &osm, &addresses, &parcels, &Overrides::default());

        assert_eq!(fixes.len(), 1);
        assert_eq!(counts.address_duplicate, 1);
        assert_eq!(
            result_for(&result, "B")["properties"]["address_state"].as_str(),
            Some("duplicate")
        );
    }
    #[test]
    fn near_join_uses_mtk_footprint() {
        let official = collection(vec![official("A", "B1", "P1", 25.001, 61.001)]);

        let nls = collection(vec![nls("A1", 25.0012, 61.0005, 25.0022, 61.0015)]);

        let osm = collection(vec![osm_way("way/100", 25.0012, 61.0005, 25.0022, 61.0015, None, None)]);

        let (result, counts, fixes) = compare(
            &official,
            &nls,
            &osm,
            &collection(Vec::new()),
            &collection(Vec::new()),
            &Overrides::default(),
        );

        assert_eq!(counts.join_inside, 0);
        assert_eq!(counts.join_near, 1);
        assert_eq!(counts.matched, 1);
        assert!(fixes.is_empty());
        assert_eq!(
            result_for(&result, "A")["properties"]["join_accuracy"].as_str(),
            Some("near")
        );
        assert_eq!(
            result_for(&result, "A")["properties"]["match_state"].as_str(),
            Some("matched")
        );
    }

    #[test]
    fn footprint_is_claimed_once() {
        let official = collection(vec![
            official("A", "B1", "P1", 25.0006, 61.0006),
            official("B", "B2", "P1", 25.0014, 61.0014),
        ]);

        let nls = collection(vec![nls("A1", 25.0005, 61.0005, 25.002, 61.002)]);

        let osm = collection(vec![osm_way("way/100", 25.0005, 61.0005, 25.002, 61.002, None, None)]);

        let (result, counts, fixes) = compare(
            &official,
            &nls,
            &osm,
            &collection(Vec::new()),
            &collection(Vec::new()),
            &Overrides::default(),
        );

        assert_eq!(counts.join_inside, 1);
        assert_eq!(counts.without_footprint, 1);
        assert_eq!(counts.matched, 1);
        assert!(fixes.is_empty());
        assert_eq!(result["features"].as_array().unwrap().len(), 1);
        let _ = result_for(&result, "B");
        assert!(
            result["features"]
                .as_array()
                .unwrap()
                .iter()
                .all(|feature| feature["properties"]["permanent_building_identifier"].as_str() != Some("A"))
        );
    }
    #[test]
    fn existing_address_is_matched_and_not_fixed() {
        let official = collection(vec![official("A", "B1", "P1", 25.001, 61.001)]);

        let nls = collection(vec![nls("A1", 25.0005, 61.0005, 25.0015, 61.0015)]);

        let osm = collection(vec![osm_way(
            "way/100",
            25.0005,
            61.0005,
            25.0015,
            61.0015,
            Some("1"),
            Some("Testikatu"),
        )]);

        let parcels = collection(vec![parcel("P1", 24.999, 60.999, 25.012, 61.012)]);

        let addresses = collection(vec![address("B1", "1", "Testikatu", 25.001, 61.001)]);

        let (result, counts, fixes) = compare(&official, &nls, &osm, &addresses, &parcels, &Overrides::default());

        assert_eq!(counts.address_matched, 1);
        assert!(fixes.is_empty());
        let a = result_for(&result, "A");
        assert_eq!(a["properties"]["address_state"].as_str(), Some("matched"));
        assert_eq!(a["properties"]["address_street_equal"].as_bool(), Some(true));
    }

    #[test]
    fn different_existing_address_is_marked_changed() {
        let official = collection(vec![official("A", "B1", "P1", 25.001, 61.001)]);

        let nls = collection(vec![nls("A1", 25.0005, 61.0005, 25.0015, 61.0015)]);

        let osm = collection(vec![osm_way(
            "way/100",
            25.0005,
            61.0005,
            25.0015,
            61.0015,
            Some("9"),
            Some("Testikatu"),
        )]);

        let parcels = collection(vec![parcel("P1", 24.999, 60.999, 25.012, 61.012)]);

        let addresses = collection(vec![address("B1", "1", "Testikatu", 25.001, 61.001)]);

        let (result, counts, fixes) = compare(&official, &nls, &osm, &addresses, &parcels, &Overrides::default());

        assert_eq!(counts.address_changed, 1);
        assert!(fixes.is_empty());
        assert_eq!(
            result_for(&result, "A")["properties"]["address_state"].as_str(),
            Some("changed")
        );
    }
}
