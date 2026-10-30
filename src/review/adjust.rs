//! Address adjustment after the first manual review of one grid cell.

use std::collections::HashSet;
use std::path::Path;

use roxmltree::Document;
use serde_json::Value;

use crate::config::Municipality;
use crate::ingest::{self, Cache};

use super::ReviewError;
use super::diff;
use super::osm_io::{self, ExistingWay};
use super::packages;
use super::state;

/// Summary of one address adjustment.
#[derive(Debug)]
pub struct AdjustOutput {
    pub kept: usize,
    pub removed: usize,
    pub placed: usize,
    pub address_fixes: usize,
}

/// Recomputes `addresses.osm` for one reviewed cell without modifying `buildings.osm`.
pub fn adjust(municipality: &Municipality, cache_dir: &Path, cell: &str) -> Result<AdjustOutput, ReviewError> {
    let cache = Cache::new(cache_dir.to_path_buf());

    let cell_dir = cache.municipality_dir(municipality).join("review").join(cell);

    if !cell_dir.is_dir() {
        return Err(ReviewError::CellNotFound(cell.to_string()));
    }

    let buildings_path = cell_dir.join("buildings.osm");

    let entries = parse_buildings_osm(&buildings_path)?;

    let kept_refs: HashSet<String> = entries
        .iter()
        .filter_map(|tags| tag_value(tags, "lacuna:ref"))
        .collect();

    let official = load(&cache, municipality, "ryhti_buildings.json")?;
    let addresses = load(&cache, municipality, "ryhti_addresses.json")?;
    let nls = load(&cache, municipality, "nls_footprints.json")?;
    let osm = load(&cache, municipality, "osm_buildings.json")?;
    let parcels = load(&cache, municipality, "parcels.json")?;

    let bbox = ingest::bbox_of_points(&official)?;

    let cells = packages::cells_of(&official, &bbox);
    let target = cells
        .iter()
        .find(|candidate| candidate.number == cell)
        .ok_or_else(|| ReviewError::CellNotFound(cell.to_string()))?;

    let state = state::load(cache_dir, municipality)?
        .ok_or_else(|| ReviewError::MissingInput(state::path(cache_dir, municipality)))?;

    let deleted: HashSet<String> = state
        .missing_by_cell
        .get(cell)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|id| !kept_refs.contains(id))
        .collect();

    // NOTE: addresses kept on the approved buildings are final, the tool only
    // recomputes the ones that are not placed there anymore
    let placed: HashSet<String> = entries
        .iter()
        .filter_map(|tags| {
            let number = tag_value(tags, "addr:housenumber")?;
            let street = tag_value(tags, "addr:street").unwrap_or_default();
            Some(diff::address_key(&number, &street))
        })
        .collect();

    // recompute only the address fixes, never the approved buildings
    let overrides = diff::Overrides {
        deleted,
        placed,
        existing_only: true,
    };

    let (result, _counts, fixes) = diff::compare(&official, &nls, &osm, &addresses, &parcels, &overrides);

    let osm_by_id = super::features_by_id(&osm);
    let owner_points = super::owner_points(&result);

    let mut address_ways: Vec<ExistingWay> = Vec::new();

    for fix in &fixes {
        let Some(feature) = osm_by_id.get(fix.way_id.as_str()) else {
            continue;
        };

        let owner_in_cell = fix
            .owner
            .as_ref()
            .and_then(|owner| owner_points.get(owner.as_str()))
            .is_some_and(|point| packages::inside_bbox(*point, &target.base_bbox));

        let way_in_cell =
            packages::feature_center(feature).is_some_and(|center| packages::inside_bbox(center, &target.base_bbox));

        if !owner_in_cell && !way_in_cell {
            continue;
        }

        if let Some(way) = packages::existing_way_from_fix(feature, fix) {
            address_ways.push(way);
        }
    }

    osm_io::save_existing_osm(&cell_dir.join("addresses.osm"), &address_ways)?;

    Ok(AdjustOutput {
        kept: entries.len(),
        removed: overrides.deleted.len(),
        placed: overrides.placed.len(),
        address_fixes: address_ways.len(),
    })
}

fn load(cache: &Cache, municipality: &Municipality, name: &str) -> Result<Value, ReviewError> {
    let path = cache.municipality_file(municipality, name);

    cache.load_json(&path)?.ok_or(ReviewError::MissingInput(path))
}

fn parse_buildings_osm(path: &Path) -> Result<Vec<Vec<(String, String)>>, ReviewError> {
    let text = std::fs::read_to_string(path)?;

    let document = Document::parse(&text)
        .map_err(|error| ReviewError::InvalidData(format!("invalid XML in buildings.osm: {error}")))?;

    let mut entries = Vec::new();

    for element in document
        .descendants()
        .filter(|node| node.has_tag_name("node") || node.has_tag_name("way"))
    {
        let tags = tags_of(element);

        if !tags.is_empty() {
            entries.push(tags);
        }
    }

    Ok(entries)
}

fn tags_of(node: roxmltree::Node) -> Vec<(String, String)> {
    node.children()
        .filter(|child| child.has_tag_name("tag"))
        .filter_map(|tag| Some((tag.attribute("k")?.to_string(), tag.attribute("v")?.to_string())))
        .collect()
}

fn tag_value(tags: &[(String, String)], key: &str) -> Option<String> {
    tags.iter()
        .find(|(tag_key, _)| tag_key == key)
        .map(|(_, value)| value.clone())
}
