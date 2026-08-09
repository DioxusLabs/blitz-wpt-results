use std::collections::HashSet;
use std::path::Path;

use chrono::{DateTime, SecondsFormat};
use serde::{Deserialize, Serialize};
use wptreport::{
    score_summary::{FocusArea, RunScores, RunSummary, ScoreSummaryReport},
    score_wpt_report,
    summarize::{RunInfoWithScores, summarize_results},
    wpt_report::{TestStatus, WptReport},
};

/// A compact on-disk representation of `ScoreSummaryReport`: each run's
/// per-area scores are stored as `[total_tests, total_score, total_subtests,
/// total_subtests_passed]` arrays (parallel to `focus_areas`) with
/// `total_score` rounded to 1dp, and each run is written on a single line.
#[derive(Serialize, Deserialize)]
struct CompactSummary {
    focus_areas: Vec<String>,
    runs: Vec<CompactRun>,
}

#[derive(Serialize, Deserialize)]
struct CompactRun {
    date: String,
    wpt_revision: String,
    product_revision: String,
    scores: Vec<(u32, f64, u32, u32)>,
}

impl From<&RunSummary> for CompactRun {
    fn from(run: &RunSummary) -> Self {
        CompactRun {
            date: run.date.clone(),
            wpt_revision: run.wpt_revision.clone(),
            product_revision: run.product_revision.clone(),
            scores: run
                .scores
                .iter()
                .map(|s| {
                    (
                        s.total_tests,
                        (s.total_score * 10.0).round() / 10.0,
                        s.total_subtests,
                        s.total_subtests_passed,
                    )
                })
                .collect(),
        }
    }
}

impl From<CompactRun> for RunSummary {
    fn from(run: CompactRun) -> Self {
        RunSummary {
            date: run.date,
            wpt_revision: run.wpt_revision,
            product_revision: run.product_revision,
            scores: run
                .scores
                .into_iter()
                .map(
                    |(total_tests, total_score, total_subtests, total_subtests_passed)| RunScores {
                        total_tests,
                        total_score,
                        total_subtests,
                        total_subtests_passed,
                    },
                )
                .collect(),
        }
    }
}

pub fn is_focus_area(area: &str) -> bool {
    let slash_count = area.chars().filter(|c| *c == '/').count();
    slash_count < 2 || (slash_count == 2 && area.starts_with("css/CSS2"))
}

/// Convert a report into a `RunInfoWithScores`, scoring only focus areas.
/// Skipped tests are stripped before scoring (matching the blitz website).
///
/// The run is dated by the blitz commit's timestamp (`commit_timestamp`),
/// falling back to the report's `time_start` (when the WPT run was executed)
/// if no commit timestamp is available.
pub fn score_report(mut report: WptReport, commit_timestamp: Option<i64>) -> RunInfoWithScores {
    report
        .results
        .retain(|test| test.status != TestStatus::Skip);

    let mut scores = score_wpt_report::<WptReport>(&report);
    scores.retain(|area, _| is_focus_area(area));

    let timestamp = commit_timestamp.unwrap_or(report.time_start as i64);
    let date = DateTime::from_timestamp(timestamp, 0)
        .expect("valid unix timestamp")
        .to_rfc3339_opts(SecondsFormat::Secs, true);

    RunInfoWithScores {
        date,
        info: report.run_info,
        scores,
    }
}

/// Load the commit-messages sidecar file (a map from blitz commit sha to the
/// first line of its commit message)
pub fn load_commit_messages(path: impl AsRef<Path>) -> std::collections::BTreeMap<String, String> {
    let Ok(contents) = std::fs::read(path) else {
        return Default::default();
    };
    serde_json::from_slice(&contents).expect("commit messages file should be valid JSON")
}

pub fn write_commit_messages(
    path: impl AsRef<Path>,
    messages: &std::collections::BTreeMap<String, String>,
) {
    let mut json = serde_json::to_string_pretty(messages).unwrap();
    json.push('\n');
    std::fs::write(path, json).unwrap();
}

pub fn load_summary(path: impl AsRef<Path>) -> Option<ScoreSummaryReport> {
    let contents = std::fs::read(path).ok()?;
    let compact: CompactSummary =
        serde_json::from_slice(&contents).expect("summary file should be valid JSON");
    Some(ScoreSummaryReport {
        focus_areas: compact.focus_areas,
        runs: compact.runs.into_iter().map(RunSummary::from).collect(),
    })
}

pub fn write_summary(path: impl AsRef<Path>, summary: &mut ScoreSummaryReport) {
    summary
        .runs
        .sort_by(|a, b| (&a.date, &a.product_revision).cmp(&(&b.date, &b.product_revision)));

    // One compact line per run, so appends produce one-line git diffs
    let mut json = String::from("{\n\"focus_areas\":");
    json.push_str(&serde_json::to_string(&summary.focus_areas).unwrap());
    json.push_str(",\n\"runs\":[\n");
    for (i, run) in summary.runs.iter().enumerate() {
        if i > 0 {
            json.push_str(",\n");
        }
        json.push_str(&serde_json::to_string(&CompactRun::from(run)).unwrap());
    }
    json.push_str("\n]}\n");
    std::fs::write(path, json).unwrap();
}

/// Append runs to an existing summary, scoring them against the summary's
/// existing focus areas. Runs whose product revision is already present are
/// skipped.
pub fn append_runs(summary: &mut ScoreSummaryReport, runs: Vec<RunInfoWithScores>) {
    let focus_areas: Vec<FocusArea> = summary
        .focus_areas
        .iter()
        .map(|area| FocusArea::from(area.as_str()))
        .collect();

    let existing_revisions: HashSet<String> = summary
        .runs
        .iter()
        .map(|run| run.product_revision.clone())
        .collect();

    let runs: Vec<RunInfoWithScores> = runs
        .into_iter()
        .filter(|run| {
            !existing_revisions.contains(run.info.browser_version.as_deref().unwrap_or(""))
        })
        .collect();

    let new_summary = summarize_results(&runs, Some(&focus_areas));
    summary.runs.extend(new_summary.runs);
}

/// Build a summary from scratch from a set of runs, deriving the focus area
/// list from the union of areas present across all runs.
pub fn build_summary(runs: Vec<RunInfoWithScores>) -> ScoreSummaryReport {
    let mut areas: Vec<String> = runs
        .iter()
        .flat_map(|run| run.scores.keys().cloned())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    areas.sort_unstable();

    let focus_areas: Vec<FocusArea> = areas
        .iter()
        .map(|area| FocusArea::from(area.as_str()))
        .collect();

    summarize_results(&runs, Some(&focus_areas))
}
