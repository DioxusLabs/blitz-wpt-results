//! wpt.fyi API client: run listing and per-run summary files.
//!
//! See https://github.com/web-platform-tests/wpt.fyi/blob/master/api/README.md

use std::collections::HashMap;
use std::io::Read;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use wptreport::SubtestCounts;

use crate::score::{RunResults, TestCounts};

const RUNS_API: &str = "https://wpt.fyi/api/runs";

/// A `TestRun` entity from `/api/runs`
#[derive(Debug, Clone, Deserialize)]
pub struct Run {
    pub id: u64,
    pub browser_version: String,
    pub full_revision_hash: String,
    pub results_url: String,
    pub time_start: String,
}

impl Run {
    pub fn start(&self) -> DateTime<Utc> {
        parse_date(&self.time_start)
    }
}

pub struct RunQuery<'a> {
    pub product: &'a str,
    pub labels: &'a [String],
    pub from: Option<&'a str>,
    pub to: Option<&'a str>,
}

/// List runs matching the query, newest first, following wpt.fyi's
/// `wpt-next-page` pagination.
///
/// The pagination token doesn't preserve `from`, so pages keep walking back in
/// time past it; we stop once a page reaches runs older than `from` and filter
/// the results client-side. `delay` is the pause between page requests.
pub fn list_runs(query: &RunQuery, delay: Duration) -> Vec<Run> {
    let mut url = format!("{RUNS_API}?product={}&max-count=500", query.product);
    if !query.labels.is_empty() {
        url.push_str(&format!("&labels={}", query.labels.join(",")));
    }
    if let Some(from) = query.from {
        url.push_str(&format!("&from={from}"));
    }
    if let Some(to) = query.to {
        url.push_str(&format!("&to={to}"));
    }

    let from = query.from.map(parse_date);
    let to = query.to.map(parse_date);

    let mut runs = Vec::new();
    loop {
        let mut response = match ureq::get(&url).call() {
            Ok(response) => response,
            // wpt.fyi returns 404 for an empty result set
            Err(ureq::Error::StatusCode(404)) => break,
            Err(err) => panic!("failed to fetch {url}: {err}"),
        };
        let next_page = response
            .headers()
            .get("wpt-next-page")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let page: Vec<Run> = response.body_mut().read_json().expect("valid runs JSON");
        let reached_from = from.is_some_and(|from| page.iter().any(|run| run.start() < from));
        runs.extend(page);

        match next_page {
            Some(token) if !reached_from => {
                url = format!("{RUNS_API}?page={token}");
                std::thread::sleep(delay);
            }
            _ => break,
        }
    }

    runs.retain(|run| {
        let start = run.start();
        from.is_none_or(|from| start >= from) && to.is_none_or(|to| start < to)
    });
    runs.sort_by_key(|run| std::cmp::Reverse(run.start()));
    runs
}

fn parse_date(date: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(date)
        .unwrap_or_else(|err| panic!("invalid RFC 3339 date {date:?}: {err}"))
        .to_utc()
}

/// A single test's entry in a `summary_v2.json.gz` file
#[derive(Deserialize)]
struct SummaryV2Entry {
    /// Abbreviated test status (`O`, `P`, `F`, `S`, ...)
    s: String,
    /// `[subtests passed, subtests total]`
    c: [u32; 2],
}

/// Fetch and parse a run's summary file (`results_url`). Skipped tests are
/// dropped.
///
/// Only the v2 format (July 2022 onwards) carries a separate test status; v1
/// summaries are `[pass, total]` with the test status folded into the counts,
/// and are scored as-is.
pub fn fetch_summary(run: &Run) -> RunResults {
    let body = ureq::get(&run.results_url)
        .call()
        .unwrap_or_else(|err| panic!("failed to fetch {}: {err}", run.results_url))
        .into_body()
        .read_to_vec()
        .expect("summary body");
    let body = maybe_gunzip(body);

    let tests = if run.results_url.contains("summary_v2") {
        let summary: HashMap<String, SummaryV2Entry> =
            serde_json::from_slice(&body).expect("valid v2 summary JSON");
        summary
            .into_iter()
            .filter(|(_, entry)| entry.s != "S")
            .map(|(path, entry)| TestCounts {
                path: strip_leading_slash(path),
                passes: entry.s == "P",
                subtests: SubtestCounts {
                    pass: entry.c[0],
                    total: entry.c[1],
                },
            })
            .collect()
    } else {
        let summary: HashMap<String, [u32; 2]> =
            serde_json::from_slice(&body).expect("valid v1 summary JSON");
        summary
            .into_iter()
            .map(|(path, [pass, total])| TestCounts {
                path: strip_leading_slash(path),
                passes: pass == total,
                subtests: SubtestCounts { pass, total },
            })
            .collect()
    };

    RunResults { tests }
}

fn strip_leading_slash(mut path: String) -> String {
    if path.starts_with('/') {
        path.remove(0);
    }
    path
}

/// GCS serves the `.json.gz` summaries with `Content-Encoding: gzip`, which
/// ureq decodes transparently, but decompress defensively if we got raw gzip.
fn maybe_gunzip(body: Vec<u8>) -> Vec<u8> {
    if body.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(body.as_slice())
            .read_to_end(&mut out)
            .expect("valid gzip");
        out
    } else {
        body
    }
}
