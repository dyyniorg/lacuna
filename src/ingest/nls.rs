//! NLS (MML) Maastotietokanta (building footprints) WFS client.

use reqwest::blocking::Client;
use roxmltree::Document;
use serde_json::{Value, json};

use crate::config::Municipality;

use super::error::IngestError;
use super::geo::{self, BBox};
use super::http;
use super::report::{Reporter, Stage};

const WFS_URL: &str = "https://inspire-wfs.maanmittauslaitos.fi/inspire-wfs/bu_mtk_polygon";
const PAGE_SIZE: usize = 1000;

pub fn footprints(
    client: &Client,
    municipality: &Municipality,
    bbox: &BBox,
    reporter: &Reporter,
) -> Result<Value, IngestError> {
    reporter.report(Stage::NlsFootprints, "Downloading NLS building footprints", Some(0));

    let (min_x, min_y, max_x, max_y) = geo::bbox_in_3067(bbox)?;
    let mut features: Vec<Value> = Vec::new();
    let mut start = 0usize;

    loop {
        let url = format!(
            "{WFS_URL}?service=WFS&version=2.0.0&request=GetFeature&typeNames=bu-core2d:Building&bbox={min_x},{min_y},{max_x},{max_y}&srsName=urn:ogc:def:crs:EPSG::4326&startIndex={start}&count={PAGE_SIZE}"
        );
        let body = http::get_text(client, &url)?;
        let document = Document::parse(&body)
            .map_err(|error| IngestError::InvalidData(format!("invalid XML from the NLS WFS: {error}")))?;

        let mut page: Vec<Value> = Vec::new();

        for member in document.descendants().filter(|node| node.has_tag_name("member")) {
            let Some(building) = member.children().find(|node| node.is_element()) else {
                continue;
            };

            let id = building
                .attributes()
                .find(|attribute| attribute.name() == "id")
                .map(|attribute| attribute.value().to_string())
                .unwrap_or_default();
            let mtk_id = id.rsplit('_').next().unwrap_or(&id).to_string();
            let rings = rings_of(building);

            if rings.is_empty() {
                continue;
            }

            let geometry = if rings.len() == 1 {
                json!({"type": "Polygon", "coordinates": vec![rings[0].clone()]})
            } else {
                json!({
                    "type": "MultiPolygon",
                    "coordinates": rings.iter().map(|ring| vec![ring.clone()]).collect::<Vec<_>>()
                })
            };

            page.push(json!({
                "type": "Feature",
                "id": format!("nls/{mtk_id}"),
                "properties": {"mtk_id": mtk_id, "source": "NLS Maastotietokanta"},
                "geometry": geometry,
            }));
        }

        let page_len = page.len();
        features.append(&mut page);

        reporter.report(
            Stage::NlsFootprints,
            format!("NLS page of {page_len} footprints, {} in total", features.len()),
            None,
        );

        start += page_len;

        if page_len < PAGE_SIZE {
            break;
        }
    }

    reporter.report(
        Stage::NlsFootprints,
        format!("Downloaded {} NLS footprints", features.len()),
        Some(100),
    );

    Ok(json!({
        "type": "FeatureCollection",
        "name": format!("{}_nls", municipality.slug()),
        "source": "NLS Maastotietokanta via INSPIRE WFS",
        "features": features,
    }))
}

fn rings_of(building: roxmltree::Node) -> Vec<Vec<Value>> {
    let mut rings = Vec::new();

    for pos_list in building.descendants().filter(|node| node.has_tag_name("posList")) {
        let mut ring: Vec<Value> = Vec::new();
        let mut numbers = pos_list.text().unwrap_or_default().split_whitespace();

        while let (Some(lat), Some(lon)) = (numbers.next(), numbers.next()) {
            let (Ok(lat), Ok(lon)) = (lat.parse::<f64>(), lon.parse::<f64>()) else {
                break;
            };
            // NOTE: the WFS serves EPSG:4326 with latitude first, GeoJSON expects longitude first
            ring.push(json!([lon, lat]));
        }

        if ring.len() >= 4 && ring.first() == ring.last() {
            rings.push(ring);
        }
    }

    rings
}
