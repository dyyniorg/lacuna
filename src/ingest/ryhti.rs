//! Ryhti building and address API client.

use reqwest::blocking::Client;
use serde_json::{Value, json};

use crate::config::Municipality;

use super::error::IngestError;
use super::http;
use super::report::{Reporter, Stage};

const RYHTI_BASE: &str = "https://paikkatiedot.ymparisto.fi/geoserver/ryhti_building/ogc/features/v1";
const PAGE_LIMIT: usize = 3000;

pub fn buildings(client: &Client, municipality: &Municipality, reporter: &Reporter) -> Result<Value, IngestError> {
    collection(
        client,
        municipality,
        "open_building",
        "ryhti_buildings",
        "official buildings",
        Stage::RyhtiBuildings,
        reporter,
    )
}

pub fn addresses(client: &Client, municipality: &Municipality, reporter: &Reporter) -> Result<Value, IngestError> {
    collection(
        client,
        municipality,
        "open_address",
        "ryhti_addresses",
        "addresses",
        Stage::RyhtiAddresses,
        reporter,
    )
}

fn collection(
    client: &Client,
    municipality: &Municipality,
    collection: &str,
    name: &str,
    label: &str,
    stage: Stage,
    reporter: &Reporter,
) -> Result<Value, IngestError> {
    reporter.report(stage, format!("Downloading {label} from Ryhti"), Some(0));

    let mut features: Vec<Value> = Vec::new();
    let mut url = format!(
        "{RYHTI_BASE}/collections/{collection}/items?filter=municipality_number={}&limit={PAGE_LIMIT}&f=application%2Fgeo%2Bjson",
        municipality.code
    );
    let mut total: Option<u64> = None;

    loop {
        let body = http::get_text(client, &url)?;
        let page: Value = serde_json::from_str(&body)?;

        if total.is_none() {
            total = page.get("numberMatched").and_then(Value::as_u64);

            if let Some(total) = total {
                reporter.report(
                    stage,
                    format!("Ryhti reports {total} {label} for {}", municipality.name),
                    Some(0),
                );
            }
        }

        let returned = page.get("numberReturned").and_then(Value::as_u64).unwrap_or(0);

        if let Some(items) = page.get("features").and_then(Value::as_array) {
            features.extend(items.iter().cloned());
        }

        let percent = total.and_then(|total| {
            if total == 0 {
                Some(100)
            } else {
                Some(((features.len() as u64 * 100) / total).min(100) as u8)
            }
        });

        reporter.report(
            stage,
            format!("Ryhti page of {returned} {label}, {} in total", features.len()),
            percent,
        );

        let next = page
            .get("links")
            .and_then(Value::as_array)
            .and_then(|links| links.iter().find(|link| link.get("rel") == Some(&json!("next"))))
            .and_then(|link| link.get("href").and_then(Value::as_str));

        match next {
            Some(href) => url = href.to_string(),
            None => break,
        }
    }

    let feature_collection = json!({
        "type": "FeatureCollection",
        "name": format!("{}_{}", municipality.slug(), name),
        "source": format!("Ryhti {collection}"),
        "features": features,
    });

    reporter.report(
        stage,
        format!(
            "Downloaded {} {label}",
            feature_collection["features"].as_array().map_or(0, Vec::len)
        ),
        Some(100),
    );

    Ok(feature_collection)
}
