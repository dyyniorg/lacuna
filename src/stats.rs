//! Review statistics and optional backend delivery.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};

use crate::config::Municipality;
use crate::ingest::user_agent;
use crate::review::CellPackage;
use crate::utils;

const OSM_CHANGESETS_URL: &str = "https://api.openstreetmap.org/api/0.6/changesets.json";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Produced {
    pub created: usize,
    pub modified: usize,
}

pub struct Config<'a> {
    pub osm_user: &'a str,
    pub url: &'a str,
    pub token: &'a str,
}

#[derive(Serialize, Deserialize)]
struct RoundStats {
    round_id: String,
    cells: BTreeMap<String, Produced>,
}

#[derive(Serialize)]
struct Event {
    version: u8,
    event_id: String,
    time: String,
    municipality: String,
    round_id: String,
    cell: String,
    osm_user: String,
    changeset_id: u64,
    produced: Produced,
    committed: Committed,
}

#[derive(Debug, Serialize)]
struct Committed {
    created: u64,
    modified: u64,
}

#[derive(Deserialize)]
struct ChangesetList {
    changesets: Vec<Changeset>,
}

#[derive(Debug, Deserialize)]
struct Changeset {
    id: u64,
    created_count: u64,
    modified_count: u64,
    deleted_count: u64,
}

pub fn write_produced(
    cache_dir: &Path,
    municipality: &Municipality,
    round_id: &str,
    cells: &[CellPackage],
) -> Result<(), String> {
    let mut stats = RoundStats {
        round_id: round_id.to_string(),
        cells: BTreeMap::new(),
    };

    for cell in cells {
        stats
            .cells
            .insert(cell.number.clone(), count_cell(cache_dir, municipality, &cell.number));
    }

    utils::save_json(&path(cache_dir, municipality), &stats, true).map_err(|error| error.to_string())
}

pub fn load_produced(
    cache_dir: &Path,
    municipality: &Municipality,
    round_id: &str,
    cell: &str,
) -> Result<Produced, String> {
    let text = fs::read_to_string(path(cache_dir, municipality)).map_err(|error| error.to_string())?;
    let stats: RoundStats = serde_json::from_str(&text).map_err(|error| error.to_string())?;

    if stats.round_id != round_id {
        return Err("Statistics are missing for the current review round".to_string());
    }

    stats
        .cells
        .get(cell)
        .copied()
        .ok_or_else(|| format!("Statistics are missing for cell {cell}"))
}

pub fn record_uploaded(
    cache_dir: &Path,
    municipality: &Municipality,
    round_id: &str,
    cell: &str,
    produced: Produced,
    config: Config<'_>,
) -> Result<(), String> {
    let changeset = latest_changeset(config.osm_user)?;
    let event = Event {
        version: 1,
        event_id: format!("{round_id}/{}/{cell}", municipality.code),
        time: utils::now_id(),
        municipality: municipality.code.to_string(),
        round_id: round_id.to_string(),
        cell: cell.to_string(),
        osm_user: config.osm_user.to_string(),
        changeset_id: changeset.id,
        produced,
        committed: Committed {
            created: changeset.created_count,
            modified: changeset.modified_count + changeset.deleted_count,
        },
    };

    let body = serde_json::to_string(&event).map_err(|error| error.to_string())?;
    append_event(cache_dir, &event)?;

    let stats_url = config.url.to_string();
    let stats_token = config.token.to_string();

    thread::spawn(move || {
        if let Err(error) = push_event(&stats_url, &stats_token, &body) {
            eprintln!("Statistics backend failed: {error}");
        }
    });

    Ok(())
}

fn path(cache_dir: &Path, municipality: &Municipality) -> PathBuf {
    cache_dir.join(municipality.slug()).join("review_stats.json")
}

fn count_cell(cache_dir: &Path, municipality: &Municipality, cell: &str) -> Produced {
    let dir = cache_dir.join(municipality.slug()).join("review").join(cell);
    let buildings = fs::read_to_string(dir.join("buildings.osm")).unwrap_or_default();
    let addresses = fs::read_to_string(dir.join("addresses.osm")).unwrap_or_default();

    Produced {
        created: buildings.matches("<node ").count() + buildings.matches("<way ").count(),
        modified: addresses.matches("<way ").count(),
    }
}

fn latest_changeset(osm_user: &str) -> Result<Changeset, String> {
    let client = Client::builder()
        .user_agent(user_agent())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| error.to_string())?;

    let response = client
        .get(OSM_CHANGESETS_URL)
        .query(&[("display_name", osm_user), ("limit", "1")])
        .send()
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;

    let text = response.text().map_err(|error| error.to_string())?;
    let list: ChangesetList = serde_json::from_str(&text).map_err(|error| error.to_string())?;

    list.changesets
        .into_iter()
        .next()
        .ok_or_else(|| format!("No changeset found for OSM user {osm_user}"))
}

fn append_event(cache_dir: &Path, event: &Event) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(cache_dir.join("stats.jsonl"))
        .map_err(|error| error.to_string())?;

    writeln!(
        file,
        "{}",
        serde_json::to_string(event).map_err(|error| error.to_string())?
    )
    .map_err(|error| error.to_string())
}

fn push_event(stats_url: &str, stats_token: &str, body: &str) -> Result<(), String> {
    let client = Client::builder()
        .user_agent(user_agent())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| error.to_string())?;

    let response = client
        .post(stats_url)
        .bearer_auth(stats_token)
        .header(CONTENT_TYPE, "application/json")
        .body(body.to_string())
        .send()
        .map_err(|error| error.to_string())?;

    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("Statistics backend returned {}", response.status()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::test_support::TempDir;

    const MUNICIPALITY: Municipality = Municipality {
        code: "291",
        name: "Kuhmoinen",
    };

    #[test]
    fn writes_and_loads_produced_counts() {
        let dir = TempDir::new("stats", "produced");
        let cell = "01_00";
        let cell_dir = dir.join(MUNICIPALITY.slug()).join("review").join(cell);
        fs::create_dir_all(&cell_dir).unwrap();
        fs::write(
            cell_dir.join("buildings.osm"),
            "<osm><node id=\"-1\"/><node id=\"-2\"/><way id=\"-3\"/></osm>",
        )
        .unwrap();
        fs::write(
            cell_dir.join("addresses.osm"),
            "<osm><node id=\"-1\"/><way id=\"100\"/></osm>",
        )
        .unwrap();

        let cells = vec![CellPackage {
            number: cell.to_string(),
            buildings: 1,
            addresses: 1,
            missing_refs: Vec::new(),
        }];

        write_produced(&dir, &MUNICIPALITY, "round", &cells).unwrap();

        assert_eq!(
            load_produced(&dir, &MUNICIPALITY, "round", cell).unwrap(),
            Produced {
                created: 3,
                modified: 1
            }
        );
    }

    #[test]
    fn appends_event_to_jsonl() {
        let dir = TempDir::new("stats", "jsonl");
        let event = Event {
            version: 1,
            event_id: "round/291/01_00".to_string(),
            time: "123".to_string(),
            municipality: "291".to_string(),
            round_id: "round".to_string(),
            cell: "01_00".to_string(),
            osm_user: "tester".to_string(),
            changeset_id: 456,
            produced: Produced {
                created: 7,
                modified: 2,
            },
            committed: Committed {
                created: 7,
                modified: 3,
            },
        };

        append_event(&dir, &event).unwrap();

        let text = fs::read_to_string(dir.join("stats.jsonl")).unwrap();
        let value: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(value["produced"]["created"], 7);
        assert_eq!(value["committed"]["modified"], 3);
    }

    #[test]
    fn parses_latest_changeset_counts() {
        let list: ChangesetList = serde_json::from_str(
            r#"{"changesets":[{"id":123,"created_count":7,"modified_count":2,"deleted_count":1}]}"#,
        )
        .unwrap();
        let changeset = list.changesets.into_iter().next().unwrap();

        assert_eq!(changeset.id, 123);
        assert_eq!(changeset.created_count, 7);
        assert_eq!(changeset.modified_count, 2);
        assert_eq!(changeset.deleted_count, 1);
    }
}
