//! OSM XML writers used by the review packages.

use std::fs;
use std::path::Path;

use serde_json::Value;

use super::ReviewError;

/// An existing OSM way with its real id, ring, and tags for JOSM.
pub struct ExistingWay {
    pub id: i64,
    pub ring: Vec<(f64, f64)>,
    pub tags: Vec<(String, String)>,
}

pub fn ensure_dir(path: &Path) -> Result<(), ReviewError> {
    fs::create_dir_all(path)?;
    Ok(())
}

pub fn save_existing_osm(path: &Path, ways: &[ExistingWay]) -> Result<(), ReviewError> {
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<osm version=\"0.6\" generator=\"lacuna\">\n");

    let mut next_id: i64 = -1;

    for way in ways {
        let ring = &way.ring;
        let closed = ring.len() >= 4 && ring.first() == ring.last();
        let vertex_count = ring.len() - usize::from(closed);
        let mut refs: Vec<i64> = Vec::new();

        for vertex in ring.iter().take(vertex_count) {
            xml.push_str(&format!(
                "  <node id=\"{next_id}\" lat=\"{}\" lon=\"{}\"/>\n",
                vertex.1, vertex.0
            ));
            refs.push(next_id);
            next_id -= 1;
        }

        if refs.is_empty() {
            continue;
        }

        if closed {
            refs.push(refs[0]);
        }

        xml.push_str(&format!("  <way id=\"{}\" version=\"1\">\n", way.id));

        for node_ref in &refs {
            xml.push_str(&format!("    <nd ref=\"{node_ref}\"/>\n"));
        }

        for (key, value) in &way.tags {
            xml.push_str(&format!(
                "    <tag k=\"{}\" v=\"{}\"/>\n",
                escape_xml(key),
                escape_xml(value)
            ));
        }

        xml.push_str("  </way>\n");
    }

    xml.push_str("</osm>\n");
    fs::write(path, xml)?;
    Ok(())
}

pub fn save_osm(path: &Path, features: &[Value]) -> Result<(), ReviewError> {
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<osm version=\"0.6\" generator=\"lacuna\">\n");

    let mut next_id: i64 = -1;

    for feature in features {
        let tags = feature
            .get("properties")
            .and_then(|props| props.get("suggested_tags"))
            .and_then(Value::as_object);
        let Some(tags) = tags else {
            continue;
        };

        let geometry = feature
            .get("geometry")
            .and_then(|geometry| geometry.get("type"))
            .and_then(Value::as_str);

        let Some(coordinates) = feature
            .get("geometry")
            .and_then(|geometry| geometry.get("coordinates"))
            .and_then(Value::as_array)
        else {
            continue;
        };

        let rings: Vec<&Value> = match geometry {
            Some("Polygon") => coordinates.iter().collect(),
            Some("MultiPolygon") => coordinates
                .iter()
                .filter_map(|part| part.as_array().and_then(|part| part.first()))
                .collect(),
            _ => continue,
        };

        for ring in rings {
            let Some(ring) = ring.as_array() else {
                continue;
            };
            let closed = ring.len() >= 4 && ring.first() == ring.last();
            let vertex_count = ring.len() - usize::from(closed);

            let mut refs: Vec<i64> = Vec::new();

            for vertex in ring.iter().take(vertex_count) {
                let Some(vertex) = vertex.as_array() else {
                    continue;
                };
                if vertex.len() < 2 {
                    continue;
                }
                xml.push_str(&format!(
                    "  <node id=\"{next_id}\" lat=\"{}\" lon=\"{}\"/>\n",
                    vertex[1], vertex[0]
                ));
                refs.push(next_id);
                next_id -= 1;
            }

            if refs.is_empty() {
                continue;
            }

            if closed {
                refs.push(refs[0]);
            }

            xml.push_str(&format!("  <way id=\"{next_id}\">\n"));
            next_id -= 1;

            for node_ref in refs {
                xml.push_str(&format!("    <nd ref=\"{node_ref}\"/>\n"));
            }

            for (key, value) in tags {
                xml.push_str(&format!(
                    "    <tag k=\"{}\" v=\"{}\"/>\n",
                    escape_xml(key),
                    escape_xml(value.as_str().unwrap_or_default())
                ));
            }

            xml.push_str("  </way>\n");
        }
    }

    xml.push_str("</osm>\n");
    fs::write(path, xml)?;

    Ok(())
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;")
}
