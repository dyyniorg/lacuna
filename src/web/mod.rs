//! Local web interface for the review workflow.

mod progress;

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

use crate::config::{self, Municipality};
use crate::ingest::{IngestClient, Progress as IngestProgress, Reporter};
use crate::review;
use crate::utils::Parameters;

use progress::ReviewProgress;

const INDEX_HTML: &str = include_str!("index.html");

/// Runs the local HTTP server.
pub fn serve(params: Parameters) -> Result<(), String> {
    let server = Server::http(params.listen()).map_err(|error| error.to_string())?;

    let state = Arc::new(Mutex::new(AppState {
        params: params.clone(),
        busy: false,
        phase: Phase::Idle,
        progress: None,
        error: None,
        last_adjust: None,
    }));

    println!("Listening on http://{}", params.listen());

    for request in server.incoming_requests() {
        let state = Arc::clone(&state);

        thread::spawn(move || {
            if let Err(error) = handle(request, state) {
                eprintln!("Web request failed: {error}");
            }
        });
    }

    Ok(())
}

struct AppState {
    params: Parameters,
    busy: bool,
    phase: Phase,
    progress: Option<ProgressView>,
    error: Option<String>,
    last_adjust: Option<AdjustView>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Idle,
    Running,
    FirstReview,
    Final,
    Error,
}

#[derive(Clone, Debug, Serialize)]
struct ProgressView {
    stage: String,
    message: String,
    percent: Option<u8>,
}

#[derive(Clone, Debug, Serialize)]
struct AdjustView {
    cell: String,
    kept: usize,
    removed: usize,
    placed: usize,
    address_fixes: usize,
}

#[derive(Clone, Debug, Serialize)]
struct MunicipalityView {
    code: &'static str,
    name: &'static str,
}

#[derive(Serialize)]
struct ConfigView {
    municipalities: Vec<MunicipalityView>,
    default_municipality: Option<String>,
    cache_max_age_days: u64,
}

#[derive(Serialize)]
struct CacheResponse {
    municipality: MunicipalityView,
    max_age_days: u64,
    entries: Vec<CacheView>,
}

#[derive(Serialize)]
struct CacheView {
    kind: String,
    path: String,
    exists: bool,
    size_bytes: u64,
    age_seconds: Option<u64>,
    fresh: bool,
}

#[derive(Serialize)]
struct SessionView {
    municipality: MunicipalityView,
    can_continue: bool,
    round_id: String,
    notes: String,
    cache_max_age_days: u64,
    cells: Vec<CellInfo>,
}

#[derive(Serialize)]
struct CellInfo {
    number: String,
    buildings: bool,
    addresses: bool,
    buildings_path: String,
    addresses_path: String,
    download_url: String,
}

#[derive(Serialize)]
struct StatusView {
    busy: bool,
    phase: Phase,
    progress: Option<ProgressView>,
    error: Option<String>,
    last_adjust: Option<AdjustView>,
}

#[derive(Deserialize)]
struct RunRequest {
    municipality: String,
    #[serde(default)]
    overwrite: bool,
}

#[derive(Deserialize)]
struct AdjustRequest {
    municipality: String,
    cell: String,
}

#[derive(Deserialize)]
struct DoneRequest {
    municipality: String,
    cell: String,
}

#[derive(Deserialize)]
struct NotesRequest {
    municipality: String,
    notes: String,
}

struct HttpResponse {
    status: u16,
    content_type: &'static str,
    body: String,
}

impl HttpResponse {
    fn html(body: &'static str) -> Self {
        Self {
            status: 200,
            content_type: "text/html; charset=utf-8",
            body: body.to_string(),
        }
    }

    fn text(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8",
            body: body.into(),
        }
    }

    fn json<T: Serialize>(status: u16, value: &T) -> Self {
        let body =
            serde_json::to_string(value).unwrap_or_else(|_| "{\"error\":\"JSON serialization failed\"}".to_string());
        Self {
            status,
            content_type: "application/json",
            body,
        }
    }

    fn error(status: u16, message: impl Into<String>) -> Self {
        Self::json(status, &json!({ "error": message.into() }))
    }
}

fn handle(mut request: Request, state: Arc<Mutex<AppState>>) -> Result<(), String> {
    let is_get = request.method() == &Method::Get;
    let is_post = request.method() == &Method::Post;

    let url = request.url().to_string();
    let (path, query) = match url.split_once('?') {
        Some((path, query)) => (path, query),
        None => (url.as_str(), ""),
    };

    let mut body = String::new();

    if is_post {
        request
            .as_reader()
            .read_to_string(&mut body)
            .map_err(|error| error.to_string())?;
    }

    let response = if is_get {
        match path {
            "/" => HttpResponse::html(INDEX_HTML),
            "/api/config" => config_response(&state),
            "/api/cache" => cache_response(&state, query),
            "/api/session" => session_response(&state, query),
            "/api/status" => status_response(&state),
            _ => HttpResponse::text(404, "Not found"),
        }
    } else if is_post {
        match path {
            "/api/run" => start_run(&state, &body),
            "/api/adjust" => start_adjust(&state, &body),
            "/api/cells/done" => mark_done(&state, &body),
            "/api/notes" => save_notes(&state, &body),
            _ => HttpResponse::text(404, "Not found"),
        }
    } else {
        HttpResponse::text(405, "Method not allowed")
    };

    respond(request, response)
}

fn respond(request: Request, response: HttpResponse) -> Result<(), String> {
    let header = Header::from_bytes(&b"Content-Type"[..], response.content_type.as_bytes())
        .map_err(|_| "Invalid response content type".to_string())?;

    request
        .respond(
            Response::from_string(response.body)
                .with_status_code(StatusCode(response.status))
                .with_header(header),
        )
        .map_err(|error| error.to_string())
}

fn config_response(state: &Arc<Mutex<AppState>>) -> HttpResponse {
    let guard = state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    let view = ConfigView {
        municipalities: config::MUNICIPALITIES.iter().map(municipality_view).collect(),
        default_municipality: guard.params.municipality().map(str::to_string),
        cache_max_age_days: cache_max_age_days(&guard.params),
    };

    drop(guard);

    HttpResponse::json(200, &view)
}

fn cache_response(state: &Arc<Mutex<AppState>>, query: &str) -> HttpResponse {
    let municipality = match municipality_from_query(query) {
        Ok(municipality) => municipality,
        Err(response) => return response,
    };

    let params = {
        let guard = state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.params.clone()
    };

    let client = match IngestClient::new(&params) {
        Ok(client) => client,
        Err(error) => return HttpResponse::error(500, error.to_string()),
    };

    let cache = match client.cache_state(municipality) {
        Ok(cache) => cache,
        Err(error) => return HttpResponse::error(500, error.to_string()),
    };

    let entries = cache
        .entries
        .into_iter()
        .map(|entry| CacheView {
            kind: entry.kind.label().to_string(),
            path: absolute_path(&entry.path),
            exists: entry.exists,
            size_bytes: entry.size_bytes,
            age_seconds: entry.age.map(|age| age.as_secs()),
            fresh: entry.fresh,
        })
        .collect();

    HttpResponse::json(
        200,
        &CacheResponse {
            municipality: municipality_view(municipality),
            max_age_days: cache_max_age_days(&params),
            entries,
        },
    )
}

fn session_response(state: &Arc<Mutex<AppState>>, query: &str) -> HttpResponse {
    let municipality = match municipality_from_query(query) {
        Ok(municipality) => municipality,
        Err(response) => return response,
    };

    let params = {
        let guard = state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.params.clone()
    };

    let cells = match review::review_cells(municipality, params.cache_dir()) {
        Ok(cells) => cells,
        Err(error) => return HttpResponse::error(500, error.to_string()),
    };

    if cells.is_empty() {
        return HttpResponse::json(
            200,
            &SessionView {
                municipality: municipality_view(municipality),
                can_continue: false,
                round_id: String::new(),
                notes: String::new(),
                cache_max_age_days: cache_max_age_days(&params),
                cells: Vec::new(),
            },
        );
    }

    let progress = match load_or_create_progress(&params, municipality) {
        Ok(progress) => progress,
        Err(error) => return HttpResponse::error(500, error),
    };

    let cell_infos = cells
        .iter()
        .map(|cell| cell_info(&params, municipality, cell, &progress))
        .collect();

    HttpResponse::json(
        200,
        &SessionView {
            municipality: municipality_view(municipality),
            can_continue: true,
            round_id: progress.round_id,
            notes: progress.notes,
            cache_max_age_days: cache_max_age_days(&params),
            cells: cell_infos,
        },
    )
}

fn status_response(state: &Arc<Mutex<AppState>>) -> HttpResponse {
    let guard = state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    let view = StatusView {
        busy: guard.busy,
        phase: guard.phase,
        progress: guard.progress.clone(),
        error: guard.error.clone(),
        last_adjust: guard.last_adjust.clone(),
    };

    drop(guard);

    HttpResponse::json(200, &view)
}

fn start_run(state: &Arc<Mutex<AppState>>, body: &str) -> HttpResponse {
    let request: RunRequest = match serde_json::from_str(body) {
        Ok(request) => request,
        Err(error) => return HttpResponse::error(400, format!("Invalid request: {error}")),
    };

    let municipality = match config::municipality(&request.municipality) {
        Some(municipality) => municipality,
        None => return HttpResponse::error(400, "Unknown municipality"),
    };

    let allow_overwrite = {
        let guard = state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        request.overwrite || guard.params.overwrite()
    };

    if !allow_overwrite {
        let existing = {
            let guard = state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            review::existing_outputs(municipality, guard.params.cache_dir())
        };

        match existing {
            Ok(paths) if !paths.is_empty() => {
                let paths: Vec<String> = paths.iter().map(|path| absolute_path(path)).collect();
                return HttpResponse::json(409, &json!({ "error": "overwrite_required", "paths": paths }));
            }
            Ok(_) => {}
            Err(error) => return HttpResponse::error(500, error.to_string()),
        }
    }

    let params = {
        let mut guard = state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        if guard.busy {
            return HttpResponse::error(409, "A job is already running");
        }

        guard.busy = true;
        guard.phase = Phase::Running;
        guard.progress = Some(ProgressView {
            stage: "Starting".to_string(),
            message: "Preparing the review run".to_string(),
            percent: None,
        });
        guard.error = None;
        guard.last_adjust = None;

        guard.params.clone()
    };

    let job_state = Arc::clone(state);
    let overwrite = allow_overwrite;

    thread::spawn(move || {
        let result = do_run(&params, &job_state, municipality, overwrite);
        let mut guard = job_state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        guard.busy = false;

        match result {
            Ok(output) => {
                guard.phase = Phase::FirstReview;
                guard.progress = Some(ProgressView {
                    stage: "Review".to_string(),
                    message: format!("{} review packages ready", output.cells.len()),
                    percent: Some(100),
                });
                guard.error = None;
            }
            Err(error) => {
                guard.phase = Phase::Error;
                guard.error = Some(error);
            }
        }
    });

    HttpResponse::json(202, &json!({ "started": true }))
}

fn do_run(
    params: &Parameters,
    state: &Arc<Mutex<AppState>>,
    municipality: &'static Municipality,
    overwrite: bool,
) -> Result<review::ReviewOutput, String> {
    if overwrite {
        let _ = fs::remove_file(progress::path(params.cache_dir(), municipality));
    }

    let reporter_state = Arc::clone(state);

    let reporter = Reporter::new(move |progress: IngestProgress| {
        let mut guard = reporter_state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        guard.progress = Some(ProgressView {
            stage: progress.stage.label().to_string(),
            message: progress.message,
            percent: progress.percent,
        });
    });

    let client = IngestClient::new(params)
        .map_err(|error| error.to_string())?
        .with_reporter(reporter);
    let data = client.fetch_all(municipality).map_err(|error| error.to_string())?;

    set_progress(state, "Review", "Comparing buildings", None);

    let options = review::ReviewOptions { overwrite };
    let output = review::run(&data, municipality, params.cache_dir(), &options).map_err(|error| error.to_string())?;

    set_progress(state, "Review", "Writing review progress", Some(100));

    let cells: Vec<String> = output.cells.iter().map(|cell| cell.number.clone()).collect();
    let progress = ReviewProgress::with_round_id(&cells, output.round_id.clone());

    progress::save(&progress::path(params.cache_dir(), municipality), &progress)?;

    Ok(output)
}

fn start_adjust(state: &Arc<Mutex<AppState>>, body: &str) -> HttpResponse {
    let request: AdjustRequest = match serde_json::from_str(body) {
        Ok(request) => request,
        Err(error) => return HttpResponse::error(400, format!("Invalid request: {error}")),
    };

    let municipality = match config::municipality(&request.municipality) {
        Some(municipality) => municipality,
        None => return HttpResponse::error(400, "Unknown municipality"),
    };

    let params = {
        let mut guard = state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        if guard.busy {
            return HttpResponse::error(409, "A job is already running");
        }

        guard.busy = true;
        guard.phase = Phase::Running;
        guard.progress = Some(ProgressView {
            stage: "Adjust".to_string(),
            message: format!("Recomputing addresses for {cell}", cell = request.cell),
            percent: None,
        });
        guard.error = None;
        guard.last_adjust = None;

        guard.params.clone()
    };

    let job_state = Arc::clone(state);
    let cell = request.cell.clone();

    thread::spawn(move || {
        let result = do_adjust(&params, municipality, &cell);
        let mut guard = job_state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.busy = false;

        match result {
            Ok(adjust) => {
                guard.phase = Phase::Final;
                guard.progress = Some(ProgressView {
                    stage: "Adjust".to_string(),
                    message: format!("{cell} adjusted", cell = adjust.cell),
                    percent: Some(100),
                });
                guard.last_adjust = Some(adjust);
                guard.error = None;
            }
            Err(error) => {
                guard.phase = Phase::Error;
                guard.error = Some(error);
            }
        }
    });

    HttpResponse::json(202, &json!({ "started": true }))
}

fn do_adjust(params: &Parameters, municipality: &'static Municipality, cell: &str) -> Result<AdjustView, String> {
    let output = review::adjust(municipality, params.cache_dir(), cell).map_err(|error| error.to_string())?;

    let mut progress = load_or_create_progress(params, municipality)?;

    progress.set_buildings(cell)?;
    progress::save(&progress::path(params.cache_dir(), municipality), &progress)?;

    Ok(AdjustView {
        cell: cell.to_string(),
        kept: output.kept,
        removed: output.removed,
        placed: output.placed,
        address_fixes: output.address_fixes,
    })
}

fn mark_done(state: &Arc<Mutex<AppState>>, body: &str) -> HttpResponse {
    let request: DoneRequest = match serde_json::from_str(body) {
        Ok(request) => request,
        Err(error) => return HttpResponse::error(400, format!("Invalid request: {error}")),
    };

    let municipality = match config::municipality(&request.municipality) {
        Some(municipality) => municipality,
        None => return HttpResponse::error(400, "Unknown municipality"),
    };

    let params = {
        let guard = state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        if guard.busy {
            return HttpResponse::error(409, "A job is already running");
        }

        guard.params.clone()
    };

    let mut progress = match load_or_create_progress(&params, municipality) {
        Ok(progress) => progress,
        Err(error) => return HttpResponse::error(500, error),
    };

    if let Err(error) = progress.set_addresses(&request.cell) {
        return HttpResponse::error(409, error);
    }

    if let Err(error) = progress::save(&progress::path(params.cache_dir(), municipality), &progress) {
        return HttpResponse::error(500, error);
    }

    HttpResponse::json(200, &json!({ "cell": request.cell, "addresses": true }))
}

fn save_notes(state: &Arc<Mutex<AppState>>, body: &str) -> HttpResponse {
    let request: NotesRequest = match serde_json::from_str(body) {
        Ok(request) => request,
        Err(error) => return HttpResponse::error(400, format!("Invalid request: {error}")),
    };

    let municipality = match config::municipality(&request.municipality) {
        Some(municipality) => municipality,
        None => return HttpResponse::error(400, "Unknown municipality"),
    };

    let params = {
        let guard = state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        if guard.busy {
            return HttpResponse::error(409, "A job is already running");
        }

        guard.params.clone()
    };

    let mut progress = match load_or_create_progress(&params, municipality) {
        Ok(progress) => progress,
        Err(error) => return HttpResponse::error(500, error),
    };

    progress.notes = request.notes;

    if let Err(error) = progress::save(&progress::path(params.cache_dir(), municipality), &progress) {
        return HttpResponse::error(500, error);
    }

    HttpResponse::json(200, &json!({ "saved": true }))
}

fn load_or_create_progress(params: &Parameters, municipality: &Municipality) -> Result<ReviewProgress, String> {
    let cells = review::review_cells(municipality, params.cache_dir()).map_err(|error| error.to_string())?;

    let path = progress::path(params.cache_dir(), municipality);
    let state = review::load_state(params.cache_dir(), municipality).map_err(|error| error.to_string())?;
    let existing = progress::load(&path)?;

    let stale = match (&existing, &state) {
        (Some(progress), Some(state)) => !state.round_id.is_empty() && progress.round_id != state.round_id,
        _ => false,
    };

    let mut progress = if stale {
        new_progress(&cells, state.as_ref())
    } else {
        existing.unwrap_or_else(|| new_progress(&cells, state.as_ref()))
    };

    let before = progress.cells.clone();
    progress.sync_cells(&cells);

    if stale || progress.cells != before || !path.exists() {
        progress::save(&path, &progress)?;
    }

    Ok(progress)
}

fn new_progress(cells: &[String], state: Option<&review::ReviewState>) -> ReviewProgress {
    match state {
        Some(state) if !state.round_id.is_empty() => ReviewProgress::with_round_id(cells, state.round_id.clone()),
        _ => ReviewProgress::new(cells),
    }
}

fn cell_info(params: &Parameters, municipality: &Municipality, cell: &str, progress: &ReviewProgress) -> CellInfo {
    let dir = params.cache_dir().join(municipality.slug()).join("review").join(cell);

    let bbox_text = fs::read_to_string(dir.join("osm_bbox.txt")).unwrap_or_default();

    let download_url = bbox_text
        .lines()
        .find(|line| line.starts_with("https://www.openstreetmap.org/"))
        .unwrap_or_default()
        .to_string();

    let state = progress.cells.get(cell).cloned().unwrap_or_default();

    CellInfo {
        number: cell.to_string(),
        buildings: state.buildings,
        addresses: state.addresses,
        buildings_path: absolute_path(&dir.join("buildings.osm")),
        addresses_path: absolute_path(&dir.join("addresses.osm")),
        download_url,
    }
}

fn municipality_from_query(query: &str) -> Result<&'static Municipality, HttpResponse> {
    let code = query_value(query, "municipality").ok_or_else(|| HttpResponse::error(400, "Missing municipality"))?;

    config::municipality(code).ok_or_else(|| HttpResponse::error(400, "Unknown municipality"))
}

fn municipality_view(municipality: &Municipality) -> MunicipalityView {
    MunicipalityView {
        code: municipality.code,
        name: municipality.name,
    }
}

fn cache_max_age_days(params: &Parameters) -> u64 {
    params.cache_max_age().as_secs() / 86_400
}

fn set_progress(state: &Arc<Mutex<AppState>>, stage: &str, message: impl Into<String>, percent: Option<u8>) {
    let mut guard = state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    guard.progress = Some(ProgressView {
        stage: stage.to_string(),
        message: message.into(),
        percent,
    });
}

fn query_value<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    for pair in query.split('&') {
        if let Some((name, value)) = pair.split_once('=') {
            if name == key {
                return Some(value);
            }
        }
    }

    None
}

fn absolute_path(path: &Path) -> String {
    if let Ok(path) = path.canonicalize() {
        return path.display().to_string();
    }

    if path.is_absolute() {
        return path.display().to_string();
    }

    std::env::current_dir()
        .map(|cwd| cwd.join(path).display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "lacuna-web-test-{label}-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));

        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("failed to create test directory");

        path
    }

    fn write_state(path: &Path, round_id: &str) {
        let state = review::ReviewState {
            round_id: round_id.to_string(),
            missing_by_cell: BTreeMap::from([("00_00".to_string(), vec!["A".to_string()])]),
        };

        fs::write(path, serde_json::to_string(&state).unwrap()).unwrap();
    }

    #[test]
    fn progress_resets_when_review_state_round_changes() {
        let dir = temp_dir("progress-reset");
        let municipality = config::municipality("291").expect("missing test municipality");
        let params = crate::utils::parse_args(&["--cache".to_string(), dir.display().to_string()]).unwrap();
        let cell = "00_00";

        let cell_dir = dir.join(municipality.slug()).join("review").join(cell);
        fs::create_dir_all(&cell_dir).unwrap();

        let state_path = dir.join(municipality.slug()).join("review_state.json");
        write_state(&state_path, "1000");

        let mut progress = load_or_create_progress(&params, municipality).unwrap();
        assert_eq!(progress.round_id, "1000");

        progress.notes = "old notes".to_string();

        progress.set_buildings(cell).unwrap();
        progress::save(&progress::path(params.cache_dir(), municipality), &progress).unwrap();

        let progress = load_or_create_progress(&params, municipality).unwrap();
        assert_eq!(progress.notes, "old notes");
        assert!(progress.cells[cell].buildings);

        write_state(&state_path, "2000");

        let progress = load_or_create_progress(&params, municipality).unwrap();
        assert_eq!(progress.round_id, "2000");
        assert!(progress.notes.is_empty());
        assert!(!progress.cells[cell].buildings);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cell_info_reads_the_osm_download_url() {
        let dir = temp_dir("cell-info");
        let municipality = config::municipality("291").expect("missing test municipality");
        let params = crate::utils::parse_args(&["--cache".to_string(), dir.display().to_string()]).unwrap();
        let cell = "00_00";
        let cell_dir = dir.join(municipality.slug()).join("review").join(cell);
        fs::create_dir_all(&cell_dir).unwrap();
        fs::write(
            cell_dir.join("osm_bbox.txt"),
            "bbox: 60.995000,24.975000,61.025000,25.025000\n\
             https://www.openstreetmap.org/#map=15/61.01000/25.00000\n",
        )
        .unwrap();

        let progress = ReviewProgress::new(&[cell.to_string()]);
        let info = cell_info(&params, municipality, cell, &progress);

        assert_eq!(
            info.download_url,
            "https://www.openstreetmap.org/#map=15/61.01000/25.00000"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
