//! Land parcel WFS client.

use reqwest::blocking::Client;
use serde_json::{Value, json};

use crate::config::Municipality;

use super::error::IngestError;
use super::geo::{self, BBox};
use super::http;
use super::report::{Reporter, Stage};

const CP_WFS: &str = "https://inspire-wfs.maanmittauslaitos.fi/inspire-wfs/cp/wfs";
const PAGE_SIZE: usize = 2000;

pub fn parcels(
    client: &Client,
    municipality: &Municipality,
    bbox: &BBox,
    reporter: &Reporter,
) -> Result<Value, IngestError> {
    reporter.report(Stage::Parcels, "Downloading land parcels");

    let (min_x, min_y, max_x, max_y) = geo::bbox_in_3067(bbox)?;
    let mut features: Vec<Value> = Vec::new();
    let mut start = 0usize;

    loop {
        let url = format!(
            "{CP_WFS}?service=WFS&version=2.0.0&request=GetFeature&typeNames=cp:CadastralParcel&bbox={min_x},{min_y},{max_x},{max_y}&srsName=urn:ogc:def:crs:EPSG::4326&outputFormat=application/json&startIndex={start}&count={PAGE_SIZE}"
        );

        let body = http::get_text(client, &url)?;
        let page: Value = serde_json::from_str(&body)?;
        let page_features = page
            .get("features")
            .and_then(Value::as_array)
            .ok_or_else(|| IngestError::InvalidData("land parcel WFS response has no features".to_string()))?;
        let page_len = page_features.len();

        features.extend(page_features.iter().cloned());

        reporter.report(
            Stage::Parcels,
            format!("Parcel page of {page_len} features, {} in total", features.len()),
        );

        start += page_len;

        if page_len < PAGE_SIZE {
            break;
        }
    }

    reporter.report(Stage::Parcels, format!("Downloaded {} parcels", features.len()));

    Ok(json!({
        "type": "FeatureCollection",
        "name": format!("{}_parcels", municipality.slug()),
        "source": "NLS land parcels via INSPIRE WFS",
        "features": features,
    }))
}
