//! Local web interface for the review workflow.

mod progress;

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

use crate::config::{self, Municipality};
use crate::ingest::{Cache, IngestClient, Progress as IngestProgress, Reporter};
use crate::review;
use crate::utils::Parameters;

use progress::ReviewProgress;

const INDEX_HTML: &str = include_str!("index.html");
const INDEX_JS: &str = include_str!("index.js");

/// Runs the local HTTP server.
pub fn serve(params: Parameters) -> Result<(), String> {
    let server = Server::http(params.listen()).map_err(|error| error.to_string())?;

    let state = Arc::new(Mutex::new(AppState {
        params: params.clone(),
        busy: false,
        progress: None,
        error: None,
        error_municipality: None,
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
    progress: Option<ProgressView>,
    error: Option<String>,
    error_municipality: Option<&'static str>,
    last_adjust: Option<AdjustView>,
}

fn lock(state: &Arc<Mutex<AppState>>) -> MutexGuard<'_, AppState> {
    state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

static PROGRESS_LOCK: Mutex<()> = Mutex::new(());

fn progress_lock() -> MutexGuard<'static, ()> {
    PROGRESS_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Clone, Debug, Serialize)]
struct ProgressView {
    stage: String,
    message: String,
    percent: Option<u8>,
}

#[derive(Clone, Debug, Serialize)]
struct AdjustView {
    municipality: &'static str,
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
}

#[derive(Serialize)]
struct CacheView {
    max_age_days: u64,
    entries: Vec<CacheEntryView>,
}

#[derive(Serialize)]
struct CacheEntryView {
    kind: String,
    path: String,
    exists: bool,
    size_bytes: u64,
    age_seconds: Option<u64>,
    fresh: bool,
}

#[derive(Serialize)]
struct SessionView {
    round_id: String,
    cache: CacheView,
    cells: Vec<CellInfo>,
}

#[derive(Serialize)]
struct CellInfo {
    number: String,
    buildings: bool,
    addresses: bool,
    skipped: bool,
    buildings_path: String,
    addresses_path: String,
    download_url: String,
}

#[derive(Serialize)]
struct StatusView {
    busy: bool,
    progress: Option<ProgressView>,
    error: Option<String>,
    error_municipality: Option<&'static str>,
    last_adjust: Option<AdjustView>,
}

#[derive(Deserialize)]
struct RunRequest {
    municipality: String,
    overwrite: bool,
}

#[derive(Deserialize)]
struct CellRequest {
    municipality: String,
    cell: String,
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

    fn javascript(body: &'static str) -> Self {
        Self {
            status: 200,
            content_type: "text/javascript; charset=utf-8",
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
            "/index.js" => HttpResponse::javascript(INDEX_JS),
            "/api/config" => config_response(&state),
            "/api/session" => session_response(&state, query),
            "/api/status" => status_response(&state),
            _ => HttpResponse::text(404, "Not found"),
        }
    } else if is_post {
        match path {
            "/api/run" => unwrap_response(start_run(&state, &body)),
            "/api/adjust" => unwrap_response(start_adjust(&state, &body)),
            "/api/cells/done" => unwrap_response(mark_done(&state, &body)),
            _ => HttpResponse::text(404, "Not found"),
        }
    } else {
        HttpResponse::text(405, "Method not allowed")
    };

    respond(request, response)
}

fn unwrap_response(result: Result<HttpResponse, HttpResponse>) -> HttpResponse {
    result.unwrap_or_else(|response| response)
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
    let guard = lock(state);

    let view = ConfigView {
        municipalities: config::MUNICIPALITIES.iter().map(municipality_view).collect(),
        default_municipality: guard.params.municipality().map(str::to_string),
    };

    drop(guard);

    HttpResponse::json(200, &view)
}

fn cache_view(params: &Parameters, municipality: &Municipality) -> Result<CacheView, String> {
    let cache = Cache::new(params.cache_dir().to_path_buf())
        .state(municipality, params.cache_max_age())
        .map_err(|error| error.to_string())?;

    let entries = cache
        .entries
        .into_iter()
        .map(|entry| CacheEntryView {
            kind: entry.kind.label().to_string(),
            path: absolute_path(&entry.path),
            exists: entry.exists,
            size_bytes: entry.size_bytes,
            age_seconds: entry.age.map(|age| age.as_secs()),
            fresh: entry.fresh,
        })
        .collect();

    Ok(CacheView {
        max_age_days: cache_max_age_days(params),
        entries,
    })
}

fn session_response(state: &Arc<Mutex<AppState>>, query: &str) -> HttpResponse {
    let municipality = match municipality_from_query(query) {
        Ok(municipality) => municipality,
        Err(response) => return response,
    };

    let params = {
        let guard = lock(state);
        guard.params.clone()
    };

    let cache = match cache_view(&params, municipality) {
        Ok(cache) => cache,
        Err(error) => return HttpResponse::error(500, error),
    };

    let cells = match review::review_cells(municipality, params.cache_dir()) {
        Ok(cells) => cells,
        Err(error) => return HttpResponse::error(500, error.to_string()),
    };

    let progress = if cells.is_empty() {
        ReviewProgress::default()
    } else {
        let _guard = progress_lock();

        match load_or_create_progress(&params, municipality) {
            Ok(progress) => progress,
            Err(error) => return HttpResponse::error(500, error),
        }
    };

    let cell_infos = cells
        .iter()
        .map(|cell| cell_info(&params, municipality, cell, &progress))
        .collect();

    HttpResponse::json(
        200,
        &SessionView {
            round_id: progress.round_id,
            cache,
            cells: cell_infos,
        },
    )
}

fn status_response(state: &Arc<Mutex<AppState>>) -> HttpResponse {
    let guard = lock(state);

    let view = StatusView {
        busy: guard.busy,
        progress: guard.progress.clone(),
        error: guard.error.clone(),
        error_municipality: guard.error_municipality,
        last_adjust: guard.last_adjust.clone(),
    };

    drop(guard);

    HttpResponse::json(200, &view)
}

fn parse_request<T: DeserializeOwned>(body: &str) -> Result<T, HttpResponse> {
    serde_json::from_str(body).map_err(|error| HttpResponse::error(400, format!("Invalid request: {error}")))
}

fn request_municipality(code: &str) -> Result<&'static Municipality, HttpResponse> {
    config::municipality(code).ok_or_else(|| HttpResponse::error(400, "Unknown municipality"))
}

fn begin_job(state: &Arc<Mutex<AppState>>, stage: &str, message: String) -> Result<Parameters, HttpResponse> {
    let mut guard = lock(state);

    if guard.busy {
        return Err(HttpResponse::error(409, "A job is already running"));
    }

    guard.busy = true;
    guard.progress = Some(ProgressView {
        stage: stage.to_string(),
        message,
        percent: None,
    });
    guard.error = None;
    guard.error_municipality = None;
    guard.last_adjust = None;

    Ok(guard.params.clone())
}

fn idle_params(state: &Arc<Mutex<AppState>>) -> Result<Parameters, HttpResponse> {
    let guard = lock(state);

    if guard.busy {
        return Err(HttpResponse::error(409, "A job is already running"));
    }

    Ok(guard.params.clone())
}

fn run_job<T>(job: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(job))
        .unwrap_or_else(|_| Err("Background job failed unexpectedly".to_string()))
}

fn finish_job<T>(
    state: &Arc<Mutex<AppState>>,
    municipality: &'static Municipality,
    result: Result<T, String>,
    on_success: impl FnOnce(&mut AppState, T),
) {
    let mut guard = lock(state);
    guard.busy = false;

    match result {
        Ok(value) => {
            on_success(&mut guard, value);
            guard.error = None;
            guard.error_municipality = None;
        }
        Err(error) => {
            guard.error = Some(error);
            guard.error_municipality = Some(municipality.code);
        }
    }
}

fn start_run(state: &Arc<Mutex<AppState>>, body: &str) -> Result<HttpResponse, HttpResponse> {
    let request: RunRequest = parse_request(body)?;
    let municipality = request_municipality(&request.municipality)?;

    let allow_overwrite = {
        let guard = lock(state);
        request.overwrite || guard.params.overwrite()
    };

    if !allow_overwrite {
        let existing = {
            let guard = lock(state);
            review::existing_outputs(municipality, guard.params.cache_dir())
        };

        match existing {
            Ok(paths) if !paths.is_empty() => {
                let paths: Vec<String> = paths.iter().map(|path| absolute_path(path)).collect();
                return Ok(HttpResponse::json(
                    409,
                    &json!({ "error": "overwrite_required", "paths": paths }),
                ));
            }
            Ok(_) => {}
            Err(error) => return Err(HttpResponse::error(500, error.to_string())),
        }
    }

    let params = begin_job(state, "Starting", "Preparing the review run".to_string())?;

    let job_state = Arc::clone(state);
    let overwrite = allow_overwrite;

    thread::spawn(move || {
        let result = run_job(|| do_run(&params, &job_state, municipality, overwrite));

        finish_job(&job_state, municipality, result, |guard, output| {
            guard.progress = Some(ProgressView {
                stage: "Review".to_string(),
                message: format!("{} review packages ready", output.cells.len()),
                percent: None,
            });
        });
    });

    Ok(HttpResponse::json(202, &json!({ "started": true })))
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
        let mut guard = lock(&reporter_state);

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

    set_progress(state, "Review", "Comparing buildings");

    let output = review::run(&data, municipality, params.cache_dir(), overwrite).map_err(|error| error.to_string())?;

    set_progress(state, "Review", "Writing review progress");

    let cells: Vec<String> = output.cells.iter().map(|cell| cell.number.clone()).collect();
    let progress = ReviewProgress::with_round_id(&cells, output.round_id.clone());
    let _guard = progress_lock();

    progress::save(&progress::path(params.cache_dir(), municipality), &progress)?;

    Ok(output)
}

fn start_adjust(state: &Arc<Mutex<AppState>>, body: &str) -> Result<HttpResponse, HttpResponse> {
    let request: CellRequest = parse_request(body)?;
    let municipality = request_municipality(&request.municipality)?;
    let params = begin_job(
        state,
        "Adjust",
        format!("Recomputing addresses for {cell}", cell = request.cell),
    )?;

    let job_state = Arc::clone(state);
    let cell = request.cell.clone();

    thread::spawn(move || {
        let result = run_job(|| do_adjust(&params, municipality, &cell));

        finish_job(&job_state, municipality, result, |guard, adjust| {
            guard.progress = Some(ProgressView {
                stage: "Adjust".to_string(),
                message: format!("{cell} adjusted", cell = adjust.cell),
                percent: None,
            });
            guard.last_adjust = Some(adjust);
        });
    });

    Ok(HttpResponse::json(202, &json!({ "started": true })))
}

fn do_adjust(params: &Parameters, municipality: &'static Municipality, cell: &str) -> Result<AdjustView, String> {
    let output = review::adjust(municipality, params.cache_dir(), cell).map_err(|error| error.to_string())?;
    let _guard = progress_lock();
    let mut progress = load_or_create_progress(params, municipality)?;

    progress.set_buildings(cell)?;
    progress::save(&progress::path(params.cache_dir(), municipality), &progress)?;

    Ok(AdjustView {
        municipality: municipality.code,
        cell: cell.to_string(),
        kept: output.kept,
        removed: output.removed,
        placed: output.placed,
        address_fixes: output.address_fixes,
    })
}

fn mark_done(state: &Arc<Mutex<AppState>>, body: &str) -> Result<HttpResponse, HttpResponse> {
    let request: CellRequest = parse_request(body)?;
    let municipality = request_municipality(&request.municipality)?;
    let params = idle_params(state)?;
    let _guard = progress_lock();

    let mut progress =
        load_or_create_progress(&params, municipality).map_err(|error| HttpResponse::error(500, error))?;

    progress
        .set_addresses(&request.cell)
        .map_err(|error| HttpResponse::error(409, error))?;

    save_progress(&params, municipality, &progress)?;

    Ok(HttpResponse::json(
        200,
        &json!({ "cell": request.cell, "addresses": true }),
    ))
}

fn save_progress(
    params: &Parameters,
    municipality: &Municipality,
    progress: &ReviewProgress,
) -> Result<(), HttpResponse> {
    progress::save(&progress::path(params.cache_dir(), municipality), progress)
        .map_err(|error| HttpResponse::error(500, error))
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

    for cell in &cells {
        if let Some(entry) = progress.cells.get_mut(cell) {
            entry.skipped = state
                .as_ref()
                .is_some_and(|state| !state.missing_by_cell.contains_key(cell));
        }
    }

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
        skipped: state.skipped,
        buildings_path: absolute_path(&dir.join("buildings.osm")),
        addresses_path: absolute_path(&dir.join("addresses.osm")),
        download_url,
    }
}

fn municipality_from_query(query: &str) -> Result<&'static Municipality, HttpResponse> {
    let code = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find_map(|(name, value)| (name == "municipality").then_some(value))
        .ok_or_else(|| HttpResponse::error(400, "Missing municipality"))?;

    request_municipality(code)
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

fn set_progress(state: &Arc<Mutex<AppState>>, stage: &str, message: impl Into<String>) {
    let mut guard = lock(state);

    guard.progress = Some(ProgressView {
        stage: stage.to_string(),
        message: message.into(),
        percent: None,
    });
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
    use std::path::Path;

    use crate::utils::test_support::TempDir;

    use super::*;

    fn write_state(path: &Path, round_id: &str) {
        let state = review::ReviewState {
            round_id: round_id.to_string(),
            missing_by_cell: BTreeMap::from([("00_00".to_string(), vec!["A".to_string()])]),
        };

        fs::write(path, serde_json::to_string(&state).unwrap()).unwrap();
    }

    #[test]
    fn job_panic_is_reported() {
        let result = run_job(|| -> Result<(), String> { panic!("boom") });
        assert_eq!(result, Err("Background job failed unexpectedly".to_string()));
    }

    #[test]
    fn session_includes_cache_state() {
        let dir = TempDir::new("web", "session-cache");
        let params = crate::utils::parse_args(&["--cache".to_string(), dir.display().to_string()]).unwrap();
        let state = Arc::new(Mutex::new(AppState {
            params,
            busy: false,
            progress: None,
            error: None,
            error_municipality: None,
            last_adjust: None,
        }));

        let response = session_response(&state, "municipality=291");
        assert_eq!(response.status, 200);

        let value: serde_json::Value = serde_json::from_str(&response.body).expect("invalid session JSON");
        let entries = value["cache"]["entries"].as_array().expect("missing cache entries");
        assert_eq!(entries.len(), 6);
        assert!(value["cells"].as_array().expect("missing cells").is_empty());
        assert_eq!(value["round_id"], "");
    }

    #[test]
    fn progress_resets_when_review_state_round_changes() {
        let dir = TempDir::new("web", "progress-reset");
        let municipality = config::municipality("291").expect("missing test municipality");
        let params = crate::utils::parse_args(&["--cache".to_string(), dir.display().to_string()]).unwrap();
        let cell = "00_00";

        let cell_dir = dir.join(municipality.slug()).join("review").join(cell);
        fs::create_dir_all(&cell_dir).unwrap();

        let state_path = dir.join(municipality.slug()).join("review_state.json");
        write_state(&state_path, "1000");

        let mut progress = load_or_create_progress(&params, municipality).unwrap();
        assert_eq!(progress.round_id, "1000");

        progress.set_buildings(cell).unwrap();
        progress::save(&progress::path(params.cache_dir(), municipality), &progress).unwrap();

        let progress = load_or_create_progress(&params, municipality).unwrap();
        assert!(progress.cells[cell].buildings);

        write_state(&state_path, "2000");

        let progress = load_or_create_progress(&params, municipality).unwrap();
        assert_eq!(progress.round_id, "2000");
        assert!(!progress.cells[cell].buildings);
    }

    #[test]
    fn cell_info_reads_the_osm_download_url() {
        let dir = TempDir::new("web", "cell-info");
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
    }
}
