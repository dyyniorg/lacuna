//! Building comparison and address assignment.
//!
//! Joins Ryhti points to NLS footprints, compares the resulting official
//! polygons against OSM buildings, and assigns official addresses to OSM
//! buildings or new building candidates.
//!
//! Point-only registry buildings are not part of the review output.

use std::collections::{HashMap, HashSet};

use geo::{Area, BooleanOps, Contains, LineString, Point, Polygon};
use rstar::{AABB, RTree, RTreeObject};
use serde_json::{Value, json};

use crate::ingest::{outer_rings, point_of_feature};
use crate::utils;

mod address;

pub use address::{AddressFix, address_key};
use address::{
    AddressInfo, AddressState, Cand, CandKind, assign_addresses, official_housenumber, set_address_state, title_case,
};

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

impl AddrCandidate {
    fn to_cand(&self) -> Cand {
        Cand {
            kind: CandKind::Osm {
                way_id: self.way_id.clone(),
                street: self.street.clone(),
            },
            area: self.area,
            key: self.way_id.clone(),
            housenumber: self.housenumber.clone(),
        }
    }
}

// Reduces an OSM feature to an address candidate.
fn osm_candidate(feature: &Value, area: f64) -> AddrCandidate {
    let tags = &feature["properties"];

    AddrCandidate {
        way_id: feature
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        housenumber: text(tags, "addr:housenumber"),
        street: text(tags, "addr:street"),
        area,
    }
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

// Text value of one JSON object key.
fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_string)
}

// Text property of a feature.
fn property(feature: &Value, key: &str) -> Option<String> {
    text(feature.get("properties")?, key)
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

type RingPolygon = (AABB<[f64; 2]>, Polygon, Vec<(f64, f64)>);

// One GeoJSON ring of [lon, lat] pairs into a geo polygon, when it is valid.
fn ring_to_polygon(ring: &Value) -> Option<RingPolygon> {
    let ring = ring.as_array()?;

    if ring.len() < 4 {
        return None;
    }

    let mut vertices: Vec<(f64, f64)> = Vec::with_capacity(ring.len());
    let mut min_lon = f64::INFINITY;
    let mut min_lat = f64::INFINITY;
    let mut max_lon = f64::NEG_INFINITY;
    let mut max_lat = f64::NEG_INFINITY;

    for vertex in ring {
        let vertex = vertex.as_array()?;
        if vertex.len() < 2 {
            return None;
        }

        let (lon, lat) = (vertex[0].as_f64()?, vertex[1].as_f64()?);
        min_lon = min_lon.min(lon);
        min_lat = min_lat.min(lat);
        max_lon = max_lon.max(lon);
        max_lat = max_lat.max(lat);
        vertices.push((lon, lat));
    }

    if vertices.first() != vertices.last() {
        return None;
    }

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

// Official addresses keyed by the building they belong to.
fn address_map(addresses: &Value) -> HashMap<String, Vec<AddressInfo>> {
    let mut addresses_by_building: HashMap<String, Vec<AddressInfo>> = HashMap::new();

    for feature in utils::features(addresses) {
        let Some(properties) = feature.get("properties") else {
            continue;
        };

        let Some(building_key) = text(properties, "building_key") else {
            continue;
        };

        let Some(housenumber) = official_housenumber(properties) else {
            continue;
        };

        let Some((lon, lat)) = point_of_feature(feature) else {
            continue;
        };

        addresses_by_building
            .entry(building_key)
            .or_default()
            .push(AddressInfo {
                housenumber,
                street: text(properties, "address_name_fin").unwrap_or_default(),
                postcode: text(properties, "postal_code").unwrap_or_default(),
                city: title_case(
                    properties
                        .get("postal_office_fin")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                ),
                full: text(properties, "address_fin").unwrap_or_default(),
                point: (lon, lat),
            });
    }

    addresses_by_building
}

// NLS footprints, one entry per outer ring.
fn build_nls_footprints(nls: &Value) -> Vec<NlsFootprint> {
    let mut footprints = Vec::new();

    for feature in utils::features(nls) {
        let Some(geometry) = feature.get("geometry") else {
            continue;
        };

        for outer in outer_rings(geometry) {
            if let Some((aabb, polygon, ring)) = ring_to_polygon(outer) {
                footprints.push(NlsFootprint {
                    aabb,
                    polygon,
                    ring,
                    geometry: geometry.clone(),
                });
            }
        }
    }

    footprints
}

// OSM polygons, one entry per outer ring.
fn build_osm_polygons(osm: &Value) -> Vec<OsmPolygon> {
    let mut polygons = Vec::new();

    for feature in utils::features(osm) {
        let Some(geometry) = feature.get("geometry") else {
            continue;
        };

        for outer in outer_rings(geometry) {
            if let Some((aabb, polygon, ring)) = ring_to_polygon(outer) {
                polygons.push(OsmPolygon {
                    aabb,
                    polygon,
                    ring,
                    feature: feature.clone(),
                });
            }
        }
    }

    polygons
}

// Land parcels scope the address comparison to the same property.
fn build_parcel_polygons(parcels: &Value) -> Vec<ParcelPoly> {
    let mut parcel_list = Vec::new();

    for feature in utils::features(parcels) {
        let Some(key) = property(feature, "nationalLandReference") else {
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

        for outer in outer_rings(geometry) {
            if let Some((aabb, polygon, _)) = ring_to_polygon(outer) {
                min_lon = min_lon.min(aabb.lower()[0]);
                min_lat = min_lat.min(aabb.lower()[1]);
                max_lon = max_lon.max(aabb.upper()[0]);
                max_lat = max_lat.max(aabb.upper()[1]);
                parts.push(polygon);
            }
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

    parcel_list
}

// Assign each OSM building to the parcel it mostly sits on.
fn build_parcel_candidates(
    osm_polygons: &[OsmPolygon],
    parcel_tree: &RTree<ParcelPoly>,
) -> (HashMap<String, Vec<AddrCandidate>>, HashMap<String, AddrCandidate>) {
    let mut parcel_candidates: HashMap<String, Vec<AddrCandidate>> = HashMap::new();
    let mut osm_candidates_by_id: HashMap<String, AddrCandidate> = HashMap::new();

    for osm_polygon in osm_polygons {
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

        let candidate = osm_candidate(&osm_polygon.feature, osm_polygon.polygon.unsigned_area());

        osm_candidates_by_id.insert(candidate.way_id.clone(), candidate.clone());

        if let Some(parcel) = assignment {
            parcel_candidates.entry(parcel.key.clone()).or_default().push(candidate);
        }
    }

    (parcel_candidates, osm_candidates_by_id)
}

// Join each official point to its footprint and resolve one-to-one claims.
fn join_footprints(points: &[(f64, f64)], footprints: &[NlsFootprint]) -> Vec<Option<(usize, bool, f64)>> {
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

    // several registry points can sit inside one mapped footprint, but the
    // footprint must not be duplicated.
    let mut claims: Vec<Option<(usize, bool, f64)>> = Vec::with_capacity(points.len());

    for (lon, lat) in points {
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
                if inside.is_none_or(|(previous, _)| {
                    footprint.polygon.unsigned_area() < footprints[previous].polygon.unsigned_area()
                }) {
                    inside = Some((candidate.index, center_distance));
                }

                continue;
            }

            let distance = distance_meters((*lon, *lat), &footprint.ring);

            if distance <= NEAR_METERS && near.is_none_or(|(_, previous)| distance < previous) {
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

    joined
}

// Review-driven changes passed into the comparison.
#[derive(Default)]
pub struct Overrides {
    // Official IDs that must not be used as new address candidates.
    pub deleted: HashSet<String>,
    // Addresses already placed on kept buildings, as address keys.
    pub placed: HashSet<String>,
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
) -> (Value, Counts, Vec<AddressFix>) {
    // official buildings that still exist
    let mut current: Vec<Value> = Vec::new();
    let mut points: Vec<(f64, f64)> = Vec::new();
    let mut building_keys: Vec<Option<String>> = Vec::new();
    let mut property_ids: Vec<Option<String>> = Vec::new();
    let mut permanent_ids: Vec<Option<String>> = Vec::new();
    let mut skipped = 0usize;

    let official_total = utils::features(official).len();

    for feature in utils::features(official) {
        let Some(point) = point_of_feature(feature) else {
            skipped += 1;
            continue;
        };

        if is_demolished(feature) || feature.get("properties").and_then(Value::as_object).is_none() {
            skipped += 1;
            continue;
        }

        current.push(feature.clone());
        points.push(point);
        building_keys.push(property(feature, "building_key"));
        property_ids.push(property(feature, "property_identifier"));
        permanent_ids.push(property(feature, "permanent_building_identifier"));
    }

    let addresses_by_building = address_map(addresses);

    let footprints = build_nls_footprints(nls);
    let nls_footprints = footprints.len();

    let osm_features = utils::features(osm).len();
    let osm_polygons = build_osm_polygons(osm);
    let osm_polygon_count = osm_polygons.len();
    let parcel_tree = RTree::bulk_load(build_parcel_polygons(parcels));

    let (parcel_candidates, osm_candidates_by_id) = build_parcel_candidates(&osm_polygons, &parcel_tree);

    let osm_tree = RTree::bulk_load(osm_polygons);

    // join each official point to its footprint, then compare against OSM
    let mut result_features: Vec<Value> = Vec::new();
    let mut join_inside = 0usize;
    let mut join_near = 0usize;
    let mut without_footprint = 0usize;
    let mut matched = 0usize;
    let mut changed = 0usize;
    let mut missing = 0usize;
    let mut building_states: Vec<String> = Vec::new();
    let mut footprint_areas: Vec<Option<f64>> = Vec::new();

    let joined = join_footprints(&points, &footprints);

    for (index, (feature, (lon, lat))) in current.iter().zip(points.iter()).enumerate() {
        let joined_here = joined[index];

        let footprint = joined_here.map(|(footprint, _, _)| &footprints[footprint]);
        let footprint_area = footprint.map(|footprint| footprint.polygon.unsigned_area());

        let join_accuracy = match joined_here {
            Some((_, true, _)) => "inside",
            Some((_, false, _)) => "near",
            None => "none",
        };

        let mut result = feature.clone();
        let state: String;

        let mut osm_way: Option<String> = None;

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

                    let smaller_area = official_area.min(candidate_area);
                    let ratio = if smaller_area > 0.0 {
                        inter_area / smaller_area
                    } else {
                        0.0
                    };

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
        properties.insert("join_accuracy".to_string(), json!(join_accuracy));
        properties.insert("official_point".to_string(), json!([lon, lat]));

        if let Some(way) = osm_way {
            properties.insert("osm_way".to_string(), json!(way));
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
    let mut address_state = AddressState::default();
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
        let mut multiple: HashSet<usize> = HashSet::new();

        for &index in indexes {
            let list = building_keys[index]
                .as_ref()
                .and_then(|key| addresses_by_building.get(key));

            match list {
                Some(list) if list.len() > 1 => {
                    multiple.insert(index);
                    address_state.counts.multiple += 1;
                    set_address_state(&mut result_features, index, "multiple");
                }
                Some(list) => {
                    if let Some(address) = list.first() {
                        addresses.push((index, address));
                    }
                }
                None => {
                    address_state.counts.none += 1;
                    set_address_state(&mut result_features, index, "no_address");
                }
            }
        }

        if addresses.is_empty() {
            continue;
        }

        let mut candidates: Vec<Cand> = Vec::new();
        let mut known_ways: HashSet<String> = HashSet::new();

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
                candidates.push(candidate.to_cand());
            }
        }

        if let Some(osm_list) = osm_list {
            for candidate in osm_list {
                if !known_ways.insert(candidate.way_id.clone()) {
                    continue;
                }

                candidates.push(candidate.to_cand());
            }
        }

        for &index in indexes {
            if multiple.contains(&index) || building_states[index] != "missing" {
                continue;
            }

            if let Some(id) = permanent_ids[index].as_ref()
                && overrides.deleted.contains(id)
            {
                continue;
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
            &mut address_state,
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
                address_state.counts.multiple += 1;
                set_address_state(&mut result_features, index, "multiple");

                continue;
            }
            Some(list) if !list.is_empty() => list.first().expect("checked"),
            _ => {
                address_state.counts.none += 1;
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

            candidates.push(osm_candidate(&candidate.feature, candidate.polygon.unsigned_area()).to_cand());
        }

        let addresses = vec![(index, address)];

        assign_addresses(
            &mut result_features,
            &addresses,
            candidates,
            &mut address_state,
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
        address_none: address_state.counts.none,
        address_matched: address_state.counts.matched,
        address_changed: address_state.counts.changed,
        address_missing: address_state.counts.missing,
        address_multiple: address_state.counts.multiple,
        address_duplicate: address_state.counts.duplicate,
    };

    let collection = json!({
        "type": "FeatureCollection",
        "features": result_features,
    });

    (collection, counts, address_state.fixes)
}
