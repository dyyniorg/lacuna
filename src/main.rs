mod config;
mod ingest;
mod review;
mod stats;
mod utils;
mod web;

use std::process;

use config::Municipality;
use utils::Parameters;

pub const PROG_NAME: &str = env!("CARGO_PKG_NAME");
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPO_URL: &str = env!("CARGO_PKG_REPOSITORY");

fn main() {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    let params = utils::parse_args(&raw_args).unwrap_or_else(|error| fatal_exit("Invalid arguments", error));

    println!("** {} v{}\n", PROG_NAME, VERSION);

    if let Some(cell) = params.adjust() {
        let municipality = selected_municipality(&params);
        let output = review::adjust(municipality, params.cache_dir(), cell)
            .unwrap_or_else(|error| fatal_exit("Address adjustment failed", error));

        println!(
            "Adjustment for {cell}: {} kept buildings, {} removed, {} addresses kept on buildings, {} address fixes",
            output.kept, output.removed, output.placed, output.address_fixes
        );
        return;
    }

    if params.review() {
        let municipality = selected_municipality(&params);

        if !params.overwrite() {
            let existing = review::existing_outputs(municipality, params.cache_dir())
                .unwrap_or_else(|error| fatal_exit("Failed to inspect review output", error));

            if !existing.is_empty() {
                fail_existing_output();
            }
        }

        let data = ingest_data(&params, municipality);

        match review::run(&data, municipality, params.cache_dir(), params.overwrite()) {
            Ok(output) => {
                if params.stats().is_some() {
                    stats::write_produced(params.cache_dir(), municipality, &output.round_id, &output.cells)
                        .unwrap_or_else(|error| fatal_exit("Statistics failed", error));
                }

                print_review_output(&output);
                return;
            }
            Err(review::ReviewError::ExistingOutput(_)) => fail_existing_output(),
            Err(error) => fatal_exit("Review failed", error),
        }
    }

    if params.fetch() {
        let municipality = selected_municipality(&params);
        let data = ingest_data(&params, municipality);

        println!("Official buildings: {}", utils::features(&data.official).len());
        println!("Official addresses: {}", utils::features(&data.addresses).len());
        println!("NLS footprints: {}", utils::features(&data.nls).len());
        println!("Land parcels: {}", utils::features(&data.parcels).len());
        println!("OSM buildings: {}", utils::features(&data.osm).len());
        println!(
            "Bounding box: {:.6},{:.6} .. {:.6},{:.6}",
            data.bbox.min_lon, data.bbox.min_lat, data.bbox.max_lon, data.bbox.max_lat
        );
        println!(
            "Bounding box size: {:.3} x {:.3} degrees",
            data.bbox.width(),
            data.bbox.height()
        );
        return;
    }

    web::serve(params).unwrap_or_else(|error| fatal_exit("Web server failed", error));
}

fn selected_municipality(params: &Parameters) -> &'static Municipality {
    let code = params
        .municipality()
        .unwrap_or_else(|| fatal_exit("Missing municipality", "Select a municipality before running"));
    config::municipality(code).unwrap_or_else(|| fatal_exit("Unknown municipality", code))
}

fn ingest_data(params: &Parameters, municipality: &Municipality) -> ingest::IngestData {
    ingest::IngestClient::new(params)
        .unwrap_or_else(|error| fatal_exit("Failed to init the ingest client", error))
        .with_reporter(ingest::Reporter::new(|progress: ingest::Progress| {
            match progress.percent {
                Some(percent) => eprintln!("{percent}% {}: {}", progress.stage.label(), progress.message),
                None => eprintln!("{}: {}", progress.stage.label(), progress.message),
            }
        }))
        .fetch_all(municipality)
        .unwrap_or_else(|error| fatal_exit("Data ingest failed", error))
}

fn print_review_output(output: &review::ReviewOutput) {
    let counts = &output.counts;
    println!("Official buildings (Ryhti, all): {}", counts.official_total);
    println!("Official buildings (current): {}", counts.official_current);
    println!("Official buildings skipped: {}", counts.official_skipped);
    println!("NLS footprints: {}", counts.nls_footprints);
    println!("Official joined (inside): {}", counts.join_inside);
    println!("Official joined (near): {}", counts.join_near);
    println!("Official without footprint: {}", counts.without_footprint);
    println!("OSM building features: {}", counts.osm_features);
    println!("OSM building polygons: {}", counts.osm_polygons);
    println!("Matched (shape overlap): {}", counts.matched);
    println!("Changed (shape differs): {}", counts.changed);
    println!("Missing (no OSM overlap): {}", counts.missing);
    println!("Buildings without address: {}", counts.address_none);
    println!("Addresses matched: {}", counts.address_matched);
    println!("Addresses changed: {}", counts.address_changed);
    println!("Addresses missing: {}", counts.address_missing);
    println!("Addresses skipped (multiple): {}", counts.address_multiple);
    println!("Addresses shared (duplicate): {}", counts.address_duplicate);
    println!("Address fixes on existing OSM ways: {}", output.address_fixes);

    let buildings: usize = output.cells.iter().map(|cell| cell.buildings).sum();
    let addresses: usize = output.cells.iter().map(|cell| cell.addresses).sum();
    let first = output.cells.first().map_or("-", |cell| cell.number.as_str());
    let last = output.cells.last().map_or("-", |cell| cell.number.as_str());

    println!(
        "Review packages: {} cells ({first} .. {last}), {buildings} buildings, {addresses} addresses",
        output.cells.len()
    );
}

fn fatal_exit(context: &str, error: impl std::fmt::Display) -> ! {
    eprintln!("{context}: {error}");
    process::exit(1);
}

fn fail_existing_output() -> ! {
    eprintln!("Review output already exists.");
    eprintln!("Re-run with --overwrite after confirming that manual review edits will be lost.");
    process::exit(1);
}
