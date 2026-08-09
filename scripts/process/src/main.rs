mod compression;
mod git;
mod github;
mod report;
mod summary;

use std::fs::canonicalize;
use std::path::{Path, PathBuf};

use compression::maybe_unzip_single_file;
use git::{git_add, git_commit, git_commit_timestamp};
use github::GithubClient;
use report::{load_existing_reports, parse_zstd_report};
use summary::{append_runs, build_summary, load_summary, score_report, write_summary};
use wptreport::summarize::RunInfoWithScores;

fn reports_dir() -> PathBuf {
    canonicalize(format!("{}/../../reports", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn summary_path() -> PathBuf {
    canonicalize(format!("{}/../..", env!("CARGO_MANIFEST_DIR")))
        .unwrap()
        .join("summary.json")
}

/// A local checkout of the blitz repository, used to look up commit
/// timestamps. Configurable via the BLITZ_REPO environment variable,
/// defaulting to a checkout named "blitz" alongside this repository.
fn blitz_repo_path() -> PathBuf {
    let path = std::env::var("BLITZ_REPO")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            canonicalize(format!("{}/../../..", env!("CARGO_MANIFEST_DIR")))
                .unwrap()
                .join("blitz")
        });
    assert!(
        path.join(".git").exists(),
        "Blitz checkout not found at {} (set BLITZ_REPO to the path of a blitz checkout)",
        path.display()
    );
    path
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("backfill") => backfill(),
        Some(cmd) => {
            eprintln!("Unknown command {cmd:?}. Usage: process [backfill]");
            std::process::exit(1);
        }
        None => process_new_artifacts(),
    }
}

/// Rebuild summary.json from scratch by scoring every report in the reports
/// directory.
fn backfill() {
    let reports_dir = reports_dir();
    let blitz_repo = blitz_repo_path();
    let report_ids = load_existing_reports(&reports_dir);
    let report_count = report_ids.len();
    println!("Backfilling summary from {report_count} reports");

    let mut runs: Vec<RunInfoWithScores> = Vec::with_capacity(report_count);
    for (idx, commit_id) in report_ids.iter().enumerate() {
        let path = reports_dir.join(format!("{commit_id}.json.zst"));
        let file = std::fs::read(&path).unwrap();
        let Ok(report) = parse_zstd_report(&file) else {
            println!("Skipping invalid report {commit_id}");
            continue;
        };
        let commit_timestamp = git_commit_timestamp(&blitz_repo, commit_id);
        if commit_timestamp.is_none() {
            println!("No commit timestamp found for {commit_id}; using WPT run time");
        }
        runs.push(score_report(report, commit_timestamp));
        if (idx + 1) % 50 == 0 {
            println!("Scored {}/{report_count} reports", idx + 1);
        }
    }

    let mut summary = build_summary(runs);
    write_summary(summary_path(), &mut summary);
    println!("Wrote summary with {} runs", summary.runs.len());
}

/// Fetch new WPT report artifacts from GitHub, commit them to the reports
/// directory, and update summary.json with their scores.
fn process_new_artifacts() {
    let reports_dir = reports_dir();
    let blitz_repo = blitz_repo_path();
    let existing_report_ids = load_existing_reports(&reports_dir);

    println!("Fetching artifacts");

    let token = std::env::var("GITHUB_TOKEN").expect("GITHUB_TOKEN environment variable not found");
    let client = GithubClient::new(&token);

    let artifact_response = client.list_artifacts(1);

    println!(
        "Showing {} of {}",
        artifact_response.artifacts.len(),
        artifact_response.total_count
    );

    let mut new_runs: Vec<RunInfoWithScores> = Vec::new();

    for artifact in &artifact_response.artifacts {
        // Skip non-main branch artifacts
        if artifact.workflow_run.head_branch != "main" {
            continue;
        }
        // Skip non-wptreport artifacts
        if !artifact.name.contains("wptreport") {
            continue;
        }

        // Stop processing once we encounter a run that has aleady been imported
        let commit_id = &artifact.workflow_run.head_sha;
        let exists = existing_report_ids.contains(&artifact.workflow_run.head_sha);
        if exists {
            break;
        }

        println!("Found new WPT report artifact:");
        println!("{}", serde_json::to_string_pretty(artifact).unwrap());

        let file = client.get_bytes(&artifact.archive_download_url);
        let file = maybe_unzip_single_file(file);
        let report = parse_zstd_report(&file);

        println!("Valid: {:?}", report.is_ok());
        let Ok(report) = report else {
            continue;
        };

        let outpath = reports_dir.join(format!("{commit_id}.json.zst"));
        std::fs::write(&outpath, file).unwrap();

        git_add(&outpath).unwrap();
        git_commit(&format!("Import WPT results for commit {commit_id}")).unwrap();

        let commit_timestamp = git_commit_timestamp(&blitz_repo, commit_id);
        if commit_timestamp.is_none() {
            println!("No commit timestamp found for {commit_id}; using WPT run time");
        }
        new_runs.push(score_report(report, commit_timestamp));
    }

    if !new_runs.is_empty() {
        update_summary(&summary_path(), new_runs);
    }
}

fn update_summary(summary_path: &Path, new_runs: Vec<RunInfoWithScores>) {
    let run_count = new_runs.len();
    let mut summary = match load_summary(summary_path) {
        Some(mut summary) => {
            append_runs(&mut summary, new_runs);
            summary
        }
        None => build_summary(new_runs),
    };
    write_summary(summary_path, &mut summary);

    println!("Updated summary with {run_count} new runs");

    git_add(summary_path).unwrap();
    git_commit(&format!("Update summary with {run_count} new runs")).unwrap();
}
