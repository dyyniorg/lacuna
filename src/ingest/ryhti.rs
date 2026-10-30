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
    reporter.report(stage, format!("Downloading {label} from Ryhti"));

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
                );
            }
        }

        let returned = page.get("numberReturned").and_then(Value::as_u64).unwrap_or(0);

        let items = page
            .get("features")
            .and_then(Value::as_array)
            .ok_or_else(|| IngestError::InvalidData(format!("Ryhti {label} response has no features")))?;
        features.extend(items.iter().cloned());

        reporter.report(
            stage,
            format!("Ryhti page of {returned} {label}, {} in total", features.len()),
        );

        let next = page
            .get("links")
            .and_then(Value::as_array)
            .and_then(|links| {
                links
                    .iter()
                    .find(|link| link.get("rel").and_then(Value::as_str) == Some("next"))
            })
            .and_then(|link| link.get("href").and_then(Value::as_str));

        match next {
            Some(href) => url = href.to_string(),
            None => break,
        }
    }

    reporter.report(stage, format!("Downloaded {} {label}", features.len()));

    Ok(json!({
        "type": "FeatureCollection",
        "name": format!("{}_{}", municipality.slug(), name),
        "source": format!("Ryhti {collection}"),
        "features": features,
    }))
}
