//! HTTP helpers with retries and download progress.

use std::fs;
use std::io::{self, BufWriter, Read, Write};
use std::path::Path;
use std::thread::sleep;
use std::time::Duration;

use reqwest::blocking::Client;

use crate::{PROG_NAME, VERSION, REPO_URL};

use super::error::IngestError;

const API_TIMEOUT: Duration = Duration::from_secs(300);
const INITIAL_DELAY: Duration = Duration::from_secs(5);
const MAX_DELAY: Duration = Duration::from_secs(160);
const MAX_ATTEMPTS: usize = 8;
const PROGRESS_STEP: u64 = 8 << 20;

fn user_agent() -> String {
    format!("{}/{} ({})", VERSION, PROG_NAME, REPO_URL)
}

pub fn api_client() -> Result<Client, IngestError> {
    Ok(Client::builder()
        .user_agent(user_agent())
        .timeout(API_TIMEOUT)
        .build()?)
}

pub fn download_client() -> Result<Client, IngestError> {
    Ok(Client::builder().user_agent(user_agent()).build()?)
}

/// Fetches a response body as text and retries transient failures.
pub fn get_text(client: &Client, url: &str) -> Result<String, IngestError> {
    let mut delay = INITIAL_DELAY;
    let mut last_error = String::new();

    for attempt in 1..=MAX_ATTEMPTS {
        match client.get(url).send() {
            Ok(response) => {
                let status = response.status();

                if status.is_success() {
                    return response.text().map_err(IngestError::from);
                }

                if status.is_client_error() && status != reqwest::StatusCode::TOO_MANY_REQUESTS {
                    return Err(IngestError::HttpStatus {
                        status,
                        url: url.to_string(),
                    });
                }

                last_error = format!("HTTP {status} for {url}");
            }
            Err(error) => {
                last_error = format!("Request error for {url}: {error}");
            }
        }

        if attempt < MAX_ATTEMPTS {
            eprintln!("{last_error}, attempt {attempt}, retrying in {delay:?}");
            sleep(delay);
            delay = (delay * 2).min(MAX_DELAY);
        }
    }

    Err(IngestError::RetriesExhausted(last_error))
}

/// Downloads a file to a temporary path and renames it into place on success.
pub fn download_file<F: FnMut(u64, Option<u64>)>(
    client: &Client,
    url: &str,
    target: &Path,
    mut progress: F,
) -> Result<(), IngestError> {
    let part = target.with_extension("part");
    let mut delay = INITIAL_DELAY;
    let mut last_error = String::new();

    for attempt in 1..=4 {
        match try_download(client, url, &part, &mut progress) {
            Ok(()) => {
                fs::rename(&part, target)?;
                return Ok(());
            }
            Err(error) => {
                last_error = error.to_string();
                eprintln!("Download failed: {last_error}, attempt {attempt}, retrying in {delay:?}");
                let _ = fs::remove_file(&part);
            }
        }

        if attempt < 4 {
            sleep(delay);
            delay *= 3;
        }
    }

    Err(IngestError::RetriesExhausted(format!("{url}: {last_error}")))
}

fn try_download<F: FnMut(u64, Option<u64>)>(
    client: &Client,
    url: &str,
    part: &Path,
    progress: &mut F,
) -> Result<(), IngestError> {
    let mut response = client.get(url).send()?;
    let status = response.status();

    if !status.is_success() {
        return Err(IngestError::HttpStatus {
            status,
            url: url.to_string(),
        });
    }

    let total = response.content_length();
    let file = fs::File::create(part)?;
    let mut writer = BufWriter::new(file);

    let done = {
        let mut reader = ProgressReader {
            inner: &mut response,
            done: 0,
            next_mark: 0,
            total,
            progress,
        };
        io::copy(&mut reader, &mut writer)?;
        reader.done
    };

    writer.flush()?;
    progress(done, total);

    Ok(())
}

struct ProgressReader<'a, F> {
    inner: &'a mut dyn Read,
    done: u64,
    next_mark: u64,
    total: Option<u64>,
    progress: &'a mut F,
}

impl<F: FnMut(u64, Option<u64>)> Read for ProgressReader<'_, F> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.inner.read(buffer)?;
        self.done += count as u64;

        if self.done >= self.next_mark {
            (self.progress)(self.done, self.total);
            self.next_mark = self.next_mark.saturating_add(PROGRESS_STEP);
        }

        Ok(count)
    }
}
