use std::collections::HashSet;
use std::path::Path;

use chrono::{DateTime, SecondsFormat};
use wptreport::{
    score_summary::{FocusArea, ScoreSummaryReport},
    score_wpt_report,
    summarize::{RunInfoWithScores, summarize_results},
    wpt_report::{TestStatus, WptReport},
};

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
    Some(serde_json::from_slice(&contents).expect("summary file should be valid JSON"))
}

pub fn write_summary(path: impl AsRef<Path>, summary: &mut ScoreSummaryReport) {
    summary
        .runs
        .sort_by(|a, b| (&a.date, &a.product_revision).cmp(&(&b.date, &b.product_revision)));

    let mut json = serde_json::to_string_pretty(summary).unwrap();
    json.push('\n');
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
