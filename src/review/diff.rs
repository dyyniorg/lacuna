//! Building comparison and address assignment.
//!
//! Joins Ryhti points to NLS footprints, compares the resulting official
//! polygons against OSM buildings, and assigns official addresses to OSM
//! buildings or new building candidates.
//!
//! Point-only registry buildings are not part of the review output.

use std::collections::HashMap;

use geo::{Area, BooleanOps, Contains, LineString, Point, Polygon};
use rstar::{AABB, RTree, RTreeObject};
use serde_json::{Value, json};

use crate::config::Municipality;

// Usage status codes that mean the building no longer exists.
const DEMOLISHED_CODES: [&str; 3] = ["06", "07", "08"];

// Minimum overlap of the smaller polygon for a building to count as matched.
const MATCH_RATIO: f64 = 0.7;

// How far a registry point may sit from its footprint for a near join.
const NEAR_METERS: f64 = 20.0;

// How far a loose official address searches for an OSM building.
const POINT_RADIUS_METERS: f64 = 15.0;

struct OsmPolygon {
    aabb: AABB<[f64; 2]>,
    polygon: Polygon,
    ring: Vec<(f64, f64)>,
    feature: Value,
}

impl RTreeObject for OsmPolygon {
    type Envelope = AABB<[f64; 2]>;

    fn envelope(&self) -> Self::Envelope {
        self.aabb
    }
}

struct NlsFootprint {
    aabb: AABB<[f64; 2]>,
    polygon: Polygon,
    ring: Vec<(f64, f64)>,
    geometry: Value,
    mtk_id: String,
}

struct FootprintRef {
    index: usize,
    aabb: AABB<[f64; 2]>,
}

impl RTreeObject for FootprintRef {
    type Envelope = AABB<[f64; 2]>;

    fn envelope(&self) -> Self::Envelope {
        self.aabb
    }
}

struct ParcelPoly {
    key: String,
    aabb: AABB<[f64; 2]>,
    parts: Vec<Polygon>,
    area: f64,
}

impl RTreeObject for ParcelPoly {
    type Envelope = AABB<[f64; 2]>;

    fn envelope(&self) -> Self::Envelope {
        self.aabb
    }
}

// An OSM building reduced to what the address comparison needs.
#[derive(Clone)]
struct AddrCandidate {
    way_id: String,
    housenumber: Option<String>,
    street: Option<String>,
    area: f64,
}

#[derive(Debug)]
pub struct Counts {
    pub official_total: usize,
    pub official_current: usize,
    pub official_skipped: usize,
    pub nls_footprints: usize,
    pub join_inside: usize,
    pub join_near: usize,
    pub without_footprint: usize,
    pub osm_features: usize,
    pub osm_polygons: usize,
    pub matched: usize,
    pub changed: usize,
    pub missing: usize,
    pub address_none: usize,
    pub address_matched: usize,
    pub address_changed: usize,
    pub address_missing: usize,
    pub address_multiple: usize,
    pub address_duplicate: usize,
}

// Point of an official feature, or None when it has no point geometry.
fn official_point(feature: &Value) -> Option<(f64, f64)> {
    let coordinates = feature.get("geometry")?.get("coordinates")?.as_array()?;

    if coordinates.len() < 2 {
        return None;
    }

    Some((coordinates[0].as_f64()?, coordinates[1].as_f64()?))
}

// Last part of a suomi.fi codelist URI, e.g. ".../code/07" -> "07".
fn status_code(feature: &Value) -> Option<&str> {
    feature
        .get("properties")?
        .get("usage_status")?
        .as_str()?
        .rsplit('/')
        .next()
}

// True when the registry says the building is gone.
fn is_demolished(feature: &Value) -> bool {
    let has_demolition_date = feature
        .get("properties")
        .and_then(|props| props.get("demolition_date"))
        .and_then(Value::as_str)
        .is_some();

    has_demolition_date
        || status_code(feature)
            .map(|code| DEMOLISHED_CODES.contains(&code))
            .unwrap_or(false)
}

// One GeoJSON ring of [lon, lat] pairs into a geo polygon, when it is valid.
fn ring_to_polygon(ring: &Value) -> Option<(AABB<[f64; 2]>, Polygon, Vec<(f64, f64)>)> {
    let ring = ring.as_array()?;

    if ring.len() < 4 {
        return None;
    }

    let mut vertices: Vec<(f64, f64)> = Vec::with_capacity(ring.len());

    for vertex in ring {
        let vertex = vertex.as_array()?;
        if vertex.len() < 2 {
            return None;
        }
        vertices.push((vertex[0].as_f64()?, vertex[1].as_f64()?));
    }

    if vertices.first() != vertices.last() {
        return None;
    }

    let min_lon = vertices.iter().map(|v| v.0).fold(f64::INFINITY, f64::min);
    let max_lon = vertices.iter().map(|v| v.0).fold(f64::NEG_INFINITY, f64::max);
    let min_lat = vertices.iter().map(|v| v.1).fold(f64::INFINITY, f64::min);
    let max_lat = vertices.iter().map(|v| v.1).fold(f64::NEG_INFINITY, f64::max);

    let polygon = Polygon::new(LineString::from(vertices.clone()), vec![]);

    Some((
        AABB::from_corners([min_lon, min_lat], [max_lon, max_lat]),
        polygon,
        vertices,
    ))
}

// Distance in meters from a point to a ring, approximated locally.
fn distance_meters(point: (f64, f64), ring: &[(f64, f64)]) -> f64 {
    let meters_per_lon = 111_320.0 * point.1.to_radians().cos();
    let mut best = f64::INFINITY;

    for i in 0..ring.len().saturating_sub(1) {
        let (x1, y1) = ring[i];
        let (x2, y2) = ring[i + 1];
        let dx = x2 - x1;
        let dy = y2 - y1;
        let t = (((point.0 - x1) * dx + (point.1 - y1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        let ddx = (point.0 - (x1 + t * dx)) * meters_per_lon;
        let ddy = (point.1 - (y1 + t * dy)) * 110_540.0;
        best = best.min(ddx.hypot(ddy));
    }

    best
}

// Overlap area of two bounding boxes, as a fallback for broken geometry.
fn bbox_overlap(a: &AABB<[f64; 2]>, b: &AABB<[f64; 2]>) -> f64 {
    let width = a.lower()[0].max(b.lower()[0]) - a.upper()[0].min(b.upper()[0]);
    let height = a.lower()[1].max(b.lower()[1]) - a.upper()[1].min(b.upper()[1]);

    (width.max(0.0) * height.max(0.0)).max(0.0)
}

struct AddressInfo {
    housenumber: String,
    street: String,
    postcode: String,
    city: String,
    full: String,
    point: (f64, f64),
}

// An OSM building that exists but has no address tags, with the official
// address that should be reviewed onto it.
pub struct AddressFix {
    pub way_id: String,
    pub housenumber: String,
    pub street: String,
    pub postcode: String,
    pub city: String,
    // Official building whose address this is.
    pub owner: Option<String>,
}

// The housenumber as OSM writes it, e.g. "4a" from parts "4" and "a".
fn official_housenumber(properties: &Value) -> Option<String> {
    let base = properties.get("number_part_of_address_number").and_then(|value| {
        value
            .as_str()
            .map(str::to_string)
            .or_else(|| value.as_u64().map(|number| number.to_string()))
    })?;

    let mut number = base;

    if let Some(letter) = properties
        .get("subdivision_letter_of_address_number")
        .and_then(Value::as_str)
    {
        number.push_str(letter);
    }

    if let Some(letter) = properties
        .get("subdivision_letter_of_address_number2")
        .and_then(Value::as_str)
    {
        number.push_str(letter);
    }

    Some(number)
}

// Key of an address, normalized number and street.
pub fn address_key(housenumber: &str, street: &str) -> String {
    format!("{} {}", normalize(housenumber), normalize(street))
}

// Lowercase alphanumerics only, for comparing address values.
fn normalize(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|character| character.is_alphanumeric())
        .collect()
}

// VTJ stores postal office names in uppercase, OSM uses normal case.
fn title_case(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut new_word = true;

    for character in text.chars() {
        if character.is_alphabetic() {
            if new_word {
                result.extend(character.to_uppercase());
                new_word = false;
            } else {
                result.extend(character.to_lowercase());
            }
        } else {
            new_word = true;
            result.push(character);
        }
    }

    result
}

// One address assignment candidate: an existing OSM building or a
// missing official building that will be added from buildings.osm.
enum CandKind {
    Osm { way_id: String, street: Option<String> },
    New { index: usize },
}

struct Cand {
    kind: CandKind,
    area: f64,
    key: String,
    housenumber: Option<String>,
}

struct AddressCounters<'a> {
    matched: &'a mut usize,
    changed: &'a mut usize,
    missing: &'a mut usize,
    duplicate: &'a mut usize,
}

fn set_address_state(result_features: &mut [Value], index: usize, state: &str) {
    if let Some(properties) = result_features[index]["properties"].as_object_mut() {
        properties.insert("address_state".to_string(), json!(state));
    }
}

fn set_address_with_info(
    result_features: &mut [Value],
    index: usize,
    state: &str,
    address: &AddressInfo,
    street_equal: Option<bool>,
) {
    if let Some(properties) = result_features[index]["properties"].as_object_mut() {
        properties.insert("address".to_string(), json!(address.full));
        properties.insert("address_state".to_string(), json!(state));
        properties.insert("address_street_equal".to_string(), json!(street_equal));
    }
}

// Assigns the official addresses of one parcel (or one loose building) to
// candidates: an address already present wins, otherwise the largest
// candidate without address information receives it.
fn assign_addresses(
    result_features: &mut [Value],
    addresses: &[(usize, &AddressInfo)],
    candidates: Vec<Cand>,
    address_fixes: &mut Vec<AddressFix>,
    counters: &mut AddressCounters,
    attached: &mut std::collections::HashSet<String>,
    attached_ways: &mut std::collections::HashSet<String>,
    placed: &std::collections::HashSet<String>,
    existing_only: bool,
    permanent_ids: &[Option<String>],
) {
    let mut used = vec![false; candidates.len()];
    for (owner, address) in addresses {
        // NOTE: one address string must not be attached twice, several official
        // records can share one address
        let address_key = address_key(&address.housenumber, &address.street);

        // check: a kept building already carries this address (if yes, leave it alone)
        if placed.contains(&address_key) {
            *counters.matched += 1;
            set_address_with_info(result_features, *owner, "matched", address, None);

            continue;
        }

        // NOTE: matching does not consume a candidate: several official records
        // can share one address, and the address only needs to exist once
        let match_candidate = candidates.iter().find(|candidate| {
            candidate
                .housenumber
                .as_ref()
                .map_or(false, |number| normalize(number) == normalize(&address.housenumber))
        });

        if let Some(candidate) = match_candidate {
            let street_equal = match &candidate.kind {
                CandKind::Osm { street, .. } => street
                    .as_ref()
                    .map(|street| normalize(street) == normalize(&address.street)),
                CandKind::New { .. } => None,
            };

            *counters.matched += 1;
            set_address_with_info(result_features, *owner, "matched", address, street_equal);

            continue;
        }

        // check: the same address already being attached for another record
        if attached.contains(&address_key) {
            *counters.duplicate += 1;
            set_address_with_info(result_features, *owner, "duplicate", address, None);

            continue;
        }
        let best = candidates
            .iter()
            .enumerate()
            .filter(|(index, candidate)| {
                !used[*index]
                    && candidate.housenumber.is_none()
                    && match &candidate.kind {
                        CandKind::Osm { way_id, .. } => !attached_ways.contains(way_id),
                        CandKind::New { .. } => !existing_only,
                    }
            })
            .max_by(|(_, left), (_, right)| left.area.total_cmp(&right.area).then_with(|| right.key.cmp(&left.key)))
            .map(|(index, _)| index);

        match best {
            Some(index) => {
                used[index] = true;

                match &candidates[index].kind {
                    CandKind::Osm { way_id, .. } => {
                        attached_ways.insert(way_id.clone());
                        address_fixes.push(AddressFix {
                            way_id: way_id.clone(),
                            housenumber: address.housenumber.clone(),
                            street: address.street.clone(),
                            postcode: address.postcode.clone(),
                            city: address.city.clone(),
                            owner: permanent_ids[*owner].clone(),
                        });
                    }
                    CandKind::New { index } => {
                        let properties = result_features[*index]["properties"]
                            .as_object_mut()
                            .expect("missing properties");

                        if let Some(suggested) = properties.get_mut("suggested_tags").and_then(Value::as_object_mut) {
                            for (key, value) in [
                                ("addr:housenumber", &address.housenumber),
                                ("addr:street", &address.street),
                                ("addr:postcode", &address.postcode),
                                ("addr:city", &address.city),
                            ] {
                                if !value.is_empty() {
                                    suggested.insert(key.to_string(), json!(value));
                                }
                            }
                        }
                    }
                }

                attached.insert(address_key);

                *counters.missing += 1;
                set_address_with_info(result_features, *owner, "missing", address, None);
            }
            None => {
                let any_addressed = candidates.iter().any(|candidate| candidate.housenumber.is_some());

                if any_addressed {
                    *counters.changed += 1;
                    set_address_with_info(result_features, *owner, "changed", address, None);
                } else {
                    *counters.missing += 1;
                    set_address_with_info(result_features, *owner, "missing", address, None);
                }
            }
        }
    }
}

// Review-driven changes passed into the comparison.
#[derive(Default)]
pub struct Overrides {
    // Official IDs that must not be used as new address candidates.
    pub deleted: std::collections::HashSet<String>,
    // Addresses already placed on kept buildings, as address keys.
    pub placed: std::collections::HashSet<String>,
    // When set, addresses only attach to existing OSM buildings.
    pub existing_only: bool,
}

// Returns the result feature collection, the counts, and the OSM
// buildings that exist but carry no address.
pub fn compare(
    official: &Value,
    nls: &Value,
    osm: &Value,
    addresses: &Value,
    parcels: &Value,
    overrides: &Overrides,
    municipality: &Municipality,
) -> (Value, Counts, Vec<AddressFix>) {
    // official buildings that still exist
    let mut current: Vec<Value> = Vec::new();
    let mut points: Vec<(f64, f64)> = Vec::new();
    let mut building_keys: Vec<Option<String>> = Vec::new();
    let mut property_ids: Vec<Option<String>> = Vec::new();
    let mut permanent_ids: Vec<Option<String>> = Vec::new();
    let mut skipped = 0usize;

    let official_total = official.get("features").and_then(Value::as_array).map_or(0, Vec::len);

    for feature in official.get("features").and_then(Value::as_array).into_iter().flatten() {
        let Some(point) = official_point(feature) else {
            skipped += 1;
            continue;
        };

        if is_demolished(feature) {
            skipped += 1;
            continue;
        }

        current.push(feature.clone());
        points.push(point);
        building_keys.push(
            feature
                .get("properties")
                .and_then(|props| props.get("building_key"))
                .and_then(Value::as_str)
                .map(str::to_string),
        );
        property_ids.push(
            feature
                .get("properties")
                .and_then(|props| props.get("property_identifier"))
                .and_then(Value::as_str)
                .map(str::to_string),
        );
        permanent_ids.push(
            feature
                .get("properties")
                .and_then(|props| props.get("permanent_building_identifier"))
                .and_then(Value::as_str)
                .map(str::to_string),
        );
    }

    // official addresses, keyed by the building they belong to
    let mut addresses_by_building: HashMap<String, Vec<AddressInfo>> = HashMap::new();

    for feature in addresses
        .get("features")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(properties) = feature.get("properties") else {
            continue;
        };

        let Some(building_key) = properties
            .get("building_key")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };

        let Some(housenumber) = official_housenumber(properties) else {
            continue;
        };

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

        addresses_by_building
            .entry(building_key)
            .or_default()
            .push(AddressInfo {
                housenumber,
                street: properties
                    .get("address_name_fin")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                postcode: properties
                    .get("postal_code")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                city: title_case(
                    properties
                        .get("postal_office_fin")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                ),
                full: properties
                    .get("address_fin")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                point: (lon, lat),
            });
    }

    // NLS footprints, indexed by their bounding boxes
    let mut footprints: Vec<NlsFootprint> = Vec::new();
    for feature in nls.get("features").and_then(Value::as_array).into_iter().flatten() {
        let Some(geometry) = feature.get("geometry") else {
            continue;
        };

        let mtk_id = feature
            .get("properties")
            .and_then(|props| props.get("mtk_id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        match geometry.get("type").and_then(Value::as_str) {
            Some("Polygon") => {
                if let Some(coordinates) = geometry.get("coordinates").and_then(Value::as_array) {
                    if let Some(outer) = coordinates.first() {
                        if let Some((aabb, polygon, ring)) = ring_to_polygon(outer) {
                            footprints.push(NlsFootprint {
                                aabb,
                                polygon,
                                ring,
                                geometry: geometry.clone(),
                                mtk_id,
                            });
                        }
                    }
                }
            }
            Some("MultiPolygon") => {
                if let Some(parts) = geometry.get("coordinates").and_then(Value::as_array) {
                    for part in parts {
                        if let Some(part) = part.as_array() {
                            if let Some(outer) = part.first() {
                                if let Some((aabb, polygon, ring)) = ring_to_polygon(outer) {
                                    footprints.push(NlsFootprint {
                                        aabb,
                                        polygon,
                                        ring,
                                        geometry: geometry.clone(),
                                        mtk_id: mtk_id.clone(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let nls_footprints = footprints.len();

    let nls_tree = RTree::bulk_load(
        footprints
            .iter()
            .enumerate()
            .map(|(index, footprint)| FootprintRef {
                index,
                aabb: footprint.aabb,
            })
            .collect::<Vec<_>>(),
    );

    // OSM polygons, indexed by their bounding boxes
    let osm_features = osm.get("features").and_then(Value::as_array).map_or(0, Vec::len);

    let mut osm_polygons: Vec<OsmPolygon> = Vec::new();

    for feature in osm.get("features").and_then(Value::as_array).into_iter().flatten() {
        let Some(geometry) = feature.get("geometry") else {
            continue;
        };

        match geometry.get("type").and_then(Value::as_str) {
            Some("Polygon") => {
                if let Some(coordinates) = geometry.get("coordinates").and_then(Value::as_array) {
                    if let Some(outer) = coordinates.first() {
                        if let Some((aabb, polygon, ring)) = ring_to_polygon(outer) {
                            osm_polygons.push(OsmPolygon {
                                aabb,
                                polygon,
                                ring,
                                feature: feature.clone(),
                            });
                        }
                    }
                }
            }
            Some("MultiPolygon") => {
                if let Some(parts) = geometry.get("coordinates").and_then(Value::as_array) {
                    for part in parts {
                        if let Some(part) = part.as_array() {
                            if let Some(outer) = part.first() {
                                if let Some((aabb, polygon, ring)) = ring_to_polygon(outer) {
                                    osm_polygons.push(OsmPolygon {
                                        aabb,
                                        polygon,
                                        ring,
                                        feature: feature.clone(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let osm_polygon_count = osm_polygons.len();

    // land parcels scope the address comparison to the same property
    let mut parcel_list: Vec<ParcelPoly> = Vec::new();

    for feature in parcels.get("features").and_then(Value::as_array).into_iter().flatten() {
        let Some(key) = feature
            .get("properties")
            .and_then(|props| props.get("nationalLandReference"))
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };

        let Some(geometry) = feature.get("geometry") else {
            continue;
        };

        let mut parts: Vec<Polygon> = Vec::new();

        let mut min_lon = f64::INFINITY;
        let mut min_lat = f64::INFINITY;
        let mut max_lon = f64::NEG_INFINITY;
        let mut max_lat = f64::NEG_INFINITY;

        match geometry.get("type").and_then(Value::as_str) {
            Some("Polygon") => {
                if let Some(coordinates) = geometry.get("coordinates").and_then(Value::as_array) {
                    if let Some(outer) = coordinates.first() {
                        if let Some((aabb, polygon, _)) = ring_to_polygon(outer) {
                            min_lon = min_lon.min(aabb.lower()[0]);
                            min_lat = min_lat.min(aabb.lower()[1]);
                            max_lon = max_lon.max(aabb.upper()[0]);
                            max_lat = max_lat.max(aabb.upper()[1]);
                            parts.push(polygon);
                        }
                    }
                }
            }
            Some("MultiPolygon") => {
                if let Some(polygons) = geometry.get("coordinates").and_then(Value::as_array) {
                    for polygon in polygons {
                        if let Some(polygon) = polygon.as_array() {
                            if let Some(outer) = polygon.first() {
                                if let Some((aabb, polygon, _)) = ring_to_polygon(outer) {
                                    min_lon = min_lon.min(aabb.lower()[0]);
                                    min_lat = min_lat.min(aabb.lower()[1]);
                                    max_lon = max_lon.max(aabb.upper()[0]);
                                    max_lat = max_lat.max(aabb.upper()[1]);
                                    parts.push(polygon);
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }

        if parts.is_empty() {
            continue;
        }

        let area: f64 = parts.iter().map(|part| part.unsigned_area()).sum();
        parcel_list.push(ParcelPoly {
            key,
            aabb: AABB::from_corners([min_lon, min_lat], [max_lon, max_lat]),
            parts,
            area,
        });
    }

    let parcel_tree = RTree::bulk_load(parcel_list);

    // assign each OSM building to the parcel it mostly sits on. ties are
    // broken explicitly, so the result does not depend on parcel order.
    let mut parcel_candidates: HashMap<String, Vec<AddrCandidate>> = HashMap::new();
    let mut osm_candidates_by_id: HashMap<String, AddrCandidate> = HashMap::new();

    for osm_polygon in &osm_polygons {
        let center = Point::new(
            (osm_polygon.aabb.lower()[0] + osm_polygon.aabb.upper()[0]) / 2.0,
            (osm_polygon.aabb.lower()[1] + osm_polygon.aabb.upper()[1]) / 2.0,
        );

        let candidates: Vec<&ParcelPoly> = parcel_tree.locate_in_envelope_intersecting(&osm_polygon.aabb).collect();

        // among the parcels containing the center, the smallest wins
        let containing = candidates
            .iter()
            .filter(|parcel| parcel.parts.iter().any(|part| part.contains(&center)))
            .min_by(|a, b| a.area.total_cmp(&b.area).then_with(|| a.key.cmp(&b.key)));

        let assignment = containing.copied().or_else(|| {
            // fall back to the parcel with the largest overlap area
            candidates
                .iter()
                .map(|parcel| {
                    let overlap: f64 = parcel
                        .parts
                        .iter()
                        .map(|part| {
                            std::panic::catch_unwind(|| part.intersection(&osm_polygon.polygon).unsigned_area())
                                .unwrap_or_else(|_| bbox_overlap(&osm_polygon.aabb, &parcel.aabb))
                        })
                        .sum();
                    (*parcel, overlap)
                })
                .max_by(|a, b| a.1.total_cmp(&b.1).then_with(|| b.0.key.cmp(&a.0.key)))
                .filter(|(_, overlap)| *overlap > 0.0)
                .map(|(parcel, _)| parcel)
        });

        let tags = &osm_polygon.feature["properties"];

        let candidate = AddrCandidate {
            way_id: osm_polygon
                .feature
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            housenumber: tags.get("addr:housenumber").and_then(Value::as_str).map(str::to_string),
            street: tags.get("addr:street").and_then(Value::as_str).map(str::to_string),
            area: osm_polygon.polygon.unsigned_area(),
        };

        osm_candidates_by_id.insert(candidate.way_id.clone(), candidate.clone());

        if let Some(parcel) = assignment {
            parcel_candidates.entry(parcel.key.clone()).or_default().push(candidate);
        }
    }

    let osm_tree = RTree::bulk_load(osm_polygons);

    // join each official point to its footprint, then compare against OSM
    let mut result_features: Vec<Value> = Vec::new();
    let mut join_inside = 0usize;
    let mut join_near = 0usize;
    let mut without_footprint = 0usize;
    let mut matched = 0usize;
    let mut changed = 0usize;
    let mut missing = 0usize;
    let mut address_none = 0usize;
    let mut address_fixes: Vec<AddressFix> = Vec::new();
    let mut address_matched = 0usize;
    let mut address_changed = 0usize;
    let mut address_missing = 0usize;
    let mut address_multiple = 0usize;
    let mut address_duplicate = 0usize;
    let mut building_states: Vec<String> = Vec::new();
    let mut footprint_areas: Vec<Option<f64>> = Vec::new();

    // claim footprints one to one. several registry points can sit inside
    // one mapped footprint, but the footprint must not be duplicated.
    let mut claims: Vec<Option<(usize, bool, f64)>> = Vec::with_capacity(points.len());

    for (lon, lat) in points.iter() {
        let point = Point::new(*lon, *lat);

        let d_lat = NEAR_METERS / 110_540.0;
        let d_lon = NEAR_METERS / (111_320.0 * lat.to_radians().cos());

        let query = AABB::from_corners([*lon - d_lon, *lat - d_lat], [*lon + d_lon, *lat + d_lat]);

        let mut inside: Option<(usize, f64)> = None;
        let mut near: Option<(usize, f64)> = None;

        for candidate in nls_tree.locate_in_envelope_intersecting(&query) {
            let footprint = &footprints[candidate.index];

            let center_lon = (footprint.aabb.lower()[0] + footprint.aabb.upper()[0]) / 2.0;
            let center_lat = (footprint.aabb.lower()[1] + footprint.aabb.upper()[1]) / 2.0;
            let center_distance = {
                let ddx = (*lon - center_lon) * 111_320.0 * lat.to_radians().cos();
                let ddy = (*lat - center_lat) * 110_540.0;
                ddx.hypot(ddy)
            };

            if footprint.polygon.contains(&point) {
                if inside.map_or(true, |(previous, _)| {
                    footprint.polygon.unsigned_area() < footprints[previous].polygon.unsigned_area()
                }) {
                    inside = Some((candidate.index, center_distance));
                }

                continue;
            }

            let distance = distance_meters((*lon, *lat), &footprint.ring);

            if distance <= NEAR_METERS && near.map_or(true, |(_, previous)| distance < previous) {
                near = Some((candidate.index, distance));
            }
        }

        claims.push(
            inside
                .or(near)
                .map(|(index, distance)| (index, inside.is_some(), distance)),
        );
    }

    // a footprint belongs to the claimant closest to its center
    let mut best_claims: HashMap<usize, (usize, bool, f64)> = HashMap::new();

    for (building, claim) in claims.iter().enumerate() {
        let Some((footprint, is_inside, distance)) = claim else {
            continue;
        };

        let better = match best_claims.get(footprint) {
            None => true,
            Some((owner, owner_inside, owner_distance)) => {
                (*is_inside && !*owner_inside)
                    || (*is_inside == *owner_inside && *distance < *owner_distance)
                    || (*is_inside == *owner_inside && *distance == *owner_distance && building < *owner)
            }
        };

        if better {
            best_claims.insert(*footprint, (building, *is_inside, *distance));
        }
    }

    let mut joined: Vec<Option<(usize, bool, f64)>> = Vec::with_capacity(points.len());

    for (building, claim) in claims.iter().enumerate() {
        let Some((footprint, is_inside, distance)) = claim else {
            joined.push(None);
            continue;
        };

        match best_claims.get(footprint) {
            Some((owner, _, _)) if *owner == building => {
                joined.push(Some((*footprint, *is_inside, *distance)));
            }
            _ => joined.push(None),
        }
    }

    for (index, (feature, (lon, lat))) in current.iter().zip(points.iter()).enumerate() {
        let joined_here = joined[index];

        let footprint = joined_here.map(|(footprint, _, _)| &footprints[footprint]);
        let footprint_area = footprint.map(|footprint| footprint.polygon.unsigned_area());

        let join_accuracy = match joined_here {
            Some((_, true, _)) => "inside",
            Some((_, false, _)) => "near",
            None => "none",
        };

        let join_distance =
            joined_here.and_then(|(_, is_inside, distance)| if is_inside { None } else { Some(distance) });

        let mut result = feature.clone();
        let state: String;

        let mut osm_way: Option<String> = None;
        let nls_building: Option<&str> = footprint.map(|footprint| footprint.mtk_id.as_str());

        let match_method: &str = "geometry";

        match footprint {
            Some(footprint) => {
                if matches!(joined_here, Some((_, true, _))) {
                    join_inside += 1;
                } else {
                    join_near += 1;
                }

                // replace the Ryhti point with the official footprint
                result["geometry"] = footprint.geometry.clone();
                let official_bbox = footprint.aabb;
                let official_area = footprint.polygon.unsigned_area();

                let mut best_overlap: f64 = 0.0;
                let mut best_way: Option<String> = None;

                for candidate in osm_tree.locate_in_envelope_intersecting(&official_bbox) {
                    let candidate_area = candidate.polygon.unsigned_area();

                    // the polygons can be broken, fall back to BBox overlap
                    let inter_area =
                        std::panic::catch_unwind(|| footprint.polygon.intersection(&candidate.polygon).unsigned_area())
                            .unwrap_or_else(|_| bbox_overlap(&official_bbox, &candidate.aabb));

                    let ratio = inter_area / official_area.min(candidate_area);

                    if ratio > best_overlap {
                        best_overlap = ratio;
                        best_way = candidate.feature.get("id").and_then(Value::as_str).map(str::to_string);
                    }
                }

                if best_overlap >= MATCH_RATIO {
                    state = "matched".to_string();
                    matched += 1;
                    osm_way = best_way;
                } else if best_overlap > 0.0 {
                    state = "changed".to_string();
                    changed += 1;
                    osm_way = best_way;
                } else {
                    state = "missing".to_string();
                    missing += 1;
                }
            }
            None => {
                // NOTE: buildings without a mapped footprint have no geometry to
                // add, they are left out of the review output entirely
                without_footprint += 1;
                state = "missing".to_string();
            }
        }

        let properties = result["properties"].as_object_mut().expect("missing properties");
        properties.insert("match_state".to_string(), json!(state));
        properties.insert("match_method".to_string(), json!(match_method));
        properties.insert("join_accuracy".to_string(), json!(join_accuracy));

        if let Some(distance) = join_distance {
            properties.insert("join_distance_m".to_string(), json!((distance * 10.0).round() / 10.0));
        }

        properties.insert("official_point".to_string(), json!([lon, lat]));

        if let Some(way) = osm_way {
            properties.insert("osm_way".to_string(), json!(way));
        }

        if let Some(nls_id) = nls_building {
            properties.insert("nls_building".to_string(), json!(nls_id));
        }

        // tags ready to paste into JOSM. address tags are added later by
        // the per-parcel assignment for the buildings that win an address
        let mut suggested = serde_json::Map::new();
        suggested.insert("building".to_string(), json!("yes"));
        suggested.insert(
            "source".to_string(),
            json!("MML Maastotietokanta; Ryhti (SYKE); CC BY 4.0"),
        );

        if let Some(permanent_id) = permanent_ids[index].as_ref() {
            suggested.insert("lacuna:ref".to_string(), json!(permanent_id));
        }

        suggested.insert("lacuna:review".to_string(), json!("yes"));
        properties.insert("suggested_tags".to_string(), json!(suggested));

        building_states.push(state.clone());
        footprint_areas.push(footprint_area);
        result_features.push(result);
    }

    // NOTE: per-parcel address assignment. the address goes to the largest
    // candidate building without address information. buildings with more
    // than one official address are skipped, OSM has no consensus for them.
    let mut attached: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut attached_ways: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut parcel_buildings: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut loose_buildings: Vec<usize> = Vec::new();

    for (index, pid) in property_ids.iter().enumerate() {
        match pid.as_deref() {
            Some(pid) => parcel_buildings.entry(pid).or_default().push(index),
            None => loose_buildings.push(index),
        }
    }

    // parcels are processed in sorted order, the shared attachment sets
    // make the order visible in the result
    let mut parcel_keys: Vec<&str> = parcel_buildings.keys().copied().collect();
    parcel_keys.sort();

    for pid in parcel_keys {
        let indexes = &parcel_buildings[pid];
        let osm_list = parcel_candidates.get(pid);
        let mut addresses: Vec<(usize, &AddressInfo)> = Vec::new();
        let mut multiple: std::collections::HashSet<usize> = std::collections::HashSet::new();

        for &index in indexes {
            let list = building_keys[index]
                .as_ref()
                .and_then(|key| addresses_by_building.get(key));

            match list {
                Some(list) if list.len() > 1 => {
                    multiple.insert(index);
                    address_multiple += 1;
                    set_address_state(&mut result_features, index, "multiple");
                }
                Some(list) => {
                    if let Some(address) = list.first() {
                        addresses.push((index, address));
                    }
                }
                None => {
                    address_none += 1;
                    set_address_state(&mut result_features, index, "no_address");
                }
            }
        }

        if addresses.is_empty() {
            continue;
        }

        let mut candidates: Vec<Cand> = Vec::new();
        let mut known_ways: std::collections::HashSet<String> = std::collections::HashSet::new();

        // OSM ways that our building match linked to this parcel, even when
        // their center fell in a neighbouring parcel
        for &index in indexes {
            let Some(way_id) = result_features[index]["properties"]
                .get("osm_way")
                .and_then(Value::as_str)
            else {
                continue;
            };

            if !known_ways.insert(way_id.to_string()) {
                continue;
            }

            if let Some(candidate) = osm_candidates_by_id.get(way_id) {
                candidates.push(Cand {
                    kind: CandKind::Osm {
                        way_id: candidate.way_id.clone(),
                        street: candidate.street.clone(),
                    },
                    area: candidate.area,
                    key: candidate.way_id.clone(),
                    housenumber: candidate.housenumber.clone(),
                });
            }
        }

        if let Some(osm_list) = osm_list {
            for candidate in osm_list {
                if !known_ways.insert(candidate.way_id.clone()) {
                    continue;
                }

                candidates.push(Cand {
                    kind: CandKind::Osm {
                        way_id: candidate.way_id.clone(),
                        street: candidate.street.clone(),
                    },
                    area: candidate.area,
                    key: candidate.way_id.clone(),
                    housenumber: candidate.housenumber.clone(),
                });
            }
        }

        for &index in indexes {
            if multiple.contains(&index) || building_states[index] != "missing" {
                continue;
            }

            if let Some(id) = permanent_ids[index].as_ref() {
                if overrides.deleted.contains(id) {
                    continue;
                }
            }

            if let Some(area) = footprint_areas[index] {
                candidates.push(Cand {
                    kind: CandKind::New { index },
                    area,
                    key: permanent_ids[index].clone().unwrap_or_else(|| index.to_string()),
                    housenumber: None,
                });
            }
        }

        addresses.sort_by(|left, right| left.1.full.cmp(&right.1.full));

        assign_addresses(
            &mut result_features,
            &addresses,
            candidates,
            &mut address_fixes,
            &mut AddressCounters {
                matched: &mut address_matched,
                changed: &mut address_changed,
                missing: &mut address_missing,
                duplicate: &mut address_duplicate,
            },
            &mut attached,
            &mut attached_ways,
            &overrides.placed,
            overrides.existing_only,
            &permanent_ids,
        );
    }

    // NOTE: buildings without a parcel identifier fall back to a radius search
    for index in loose_buildings {
        let list = building_keys[index]
            .as_ref()
            .and_then(|key| addresses_by_building.get(key));

        let address = match list {
            Some(list) if list.len() > 1 => {
                address_multiple += 1;
                set_address_state(&mut result_features, index, "multiple");

                continue;
            }
            Some(list) if !list.is_empty() => list.first().expect("checked"),
            _ => {
                address_none += 1;
                set_address_state(&mut result_features, index, "no_address");

                continue;
            }
        };

        let addr_point = Point::new(address.point.0, address.point.1);
        let d_lat = POINT_RADIUS_METERS / 110_540.0;
        let d_lon = POINT_RADIUS_METERS / (111_320.0 * address.point.1.to_radians().cos());
        let query = AABB::from_corners(
            [address.point.0 - d_lon, address.point.1 - d_lat],
            [address.point.0 + d_lon, address.point.1 + d_lat],
        );

        let mut candidates: Vec<Cand> = Vec::new();

        for candidate in osm_tree.locate_in_envelope_intersecting(&query) {
            let distance = if candidate.polygon.contains(&addr_point) {
                0.0
            } else {
                distance_meters(address.point, &candidate.ring)
            };

            if distance > POINT_RADIUS_METERS {
                continue;
            }

            let tags = &candidate.feature["properties"];
            let way_id = candidate
                .feature
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();

            candidates.push(Cand {
                kind: CandKind::Osm {
                    way_id: way_id.clone(),
                    street: tags.get("addr:street").and_then(Value::as_str).map(str::to_string),
                },
                area: candidate.polygon.unsigned_area(),
                key: way_id,
                housenumber: tags.get("addr:housenumber").and_then(Value::as_str).map(str::to_string),
            });
        }

        let addresses = vec![(index, address)];

        assign_addresses(
            &mut result_features,
            &addresses,
            candidates,
            &mut address_fixes,
            &mut AddressCounters {
                matched: &mut address_matched,
                changed: &mut address_changed,
                missing: &mut address_missing,
                duplicate: &mut address_duplicate,
            },
            &mut attached,
            &mut attached_ways,
            &overrides.placed,
            overrides.existing_only,
            &permanent_ids,
        );
    }

    // NOTE: point-only buildings are not part of the review output
    result_features.retain(|feature| feature["geometry"]["type"] != "Point");

    let counts = Counts {
        official_total,
        official_current: current.len(),
        official_skipped: skipped,
        nls_footprints,
        join_inside,
        join_near,
        without_footprint,
        osm_features,
        osm_polygons: osm_polygon_count,
        matched,
        changed,
        missing,
        address_none,
        address_matched,
        address_changed,
        address_missing,
        address_multiple,
        address_duplicate,
    };

    let collection = json!({
        "type": "FeatureCollection",
        "name": format!("{}_result", municipality.slug()),
        "source": "Ryhti points joined to NLS footprints, compared against OSM",
        "features": result_features,
    });

    (collection, counts, address_fixes)
}
