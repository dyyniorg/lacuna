//! NLS Topographic database (building footprints) WFS client.

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
    reporter.report(Stage::NlsFootprints, "Downloading NLS building footprints");

    let (min_x, min_y, max_x, max_y) = geo::bbox_in_3067(bbox)?;
    let mut features: Vec<Value> = Vec::new();
    let mut start = 0usize;

    loop {
        let url = format!(
            "{WFS_URL}?service=WFS&version=2.0.0&request=GetFeature&typeNames=bu-core2d:Building&bbox={min_x},{min_y},{max_x},{max_y}&srsName=urn:ogc:def:crs:EPSG::4326&startIndex={start}&count={PAGE_SIZE}"
        );
        let body = http::get_text(client, &url)?;
        let (mut page, returned) = parse_page(&body)?;
        let page_len = page.len();
        features.append(&mut page);

        reporter.report(
            Stage::NlsFootprints,
            format!("NLS page of {page_len} footprints, {} in total", features.len()),
        );

        start += returned;

        if returned < PAGE_SIZE {
            break;
        }
    }

    reporter.report(
        Stage::NlsFootprints,
        format!("Downloaded {} NLS footprints", features.len()),
    );

    Ok(json!({
        "type": "FeatureCollection",
        "name": format!("{}_nls", municipality.slug()),
        "source": "NLS Topographic database via INSPIRE WFS",
        "features": features,
    }))
}

fn parse_page(body: &str) -> Result<(Vec<Value>, usize), IngestError> {
    let document = Document::parse(body)
        .map_err(|error| IngestError::InvalidData(format!("invalid XML from the NLS WFS: {error}")))?;

    if document
        .descendants()
        .any(|node| node.has_tag_name("ExceptionReport") || node.has_tag_name("ServiceExceptionReport"))
    {
        return Err(IngestError::InvalidData(
            "NLS WFS returned an exception report".to_string(),
        ));
    }

    let mut page: Vec<Value> = Vec::new();
    let mut returned = 0usize;

    for member in document.descendants().filter(|node| node.has_tag_name("member")) {
        returned += 1;

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
            "properties": {"mtk_id": mtk_id, "source": "NLS Topographic database"},
            "geometry": geometry,
        }));
    }

    Ok((page, returned))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_exception_reports() {
        let xml = r#"<ows:ExceptionReport xmlns:ows="http://www.opengis.net/ows/1.1"/>"#;
        assert!(parse_page(xml).is_err());
    }

    #[test]
    fn counts_returned_members_even_when_skipped() {
        let xml = r#"
            <FeatureCollection xmlns="http://www.opengis.net/wfs/2.0">
                <member>
                    <bu:Building xmlns:bu="urn:test" gml:id="a_123" xmlns:gml="http://www.opengis.net/gml/3.2">
                        <gml:posList>1 2 3 4 5 6 7 8 1 2</gml:posList>
                    </bu:Building>
                </member>
                <member><bu:Empty xmlns:bu="urn:test"/></member>
            </FeatureCollection>
        "#;

        let (page, returned) = parse_page(xml).expect("valid page");

        assert_eq!(returned, 2);
        assert_eq!(page.len(), 1);
    }
}
