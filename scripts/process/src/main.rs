mod compression;
mod github;

use std::{
    collections::{BTreeMap, HashSet},
    io::Cursor,
};

use compression::{maybe_unzip_single_file, zstd_decode, zstd_encode};
use github::GithubClient;
use wptreport::{
    AreaScores,
    score_summary::FocusArea,
    score_wpt_report,
    summarize::{RunInfoWithScores, summarize_results},
    wpt_report::WptReport,
};

fn main() {
    println!("Fetching artifacts");

    let token = std::env::var("GITHUB_TOKEN").expect("GITHUB_TOKEN environment variable not found");
    let client = GithubClient::new(&token);

    let artifact_response = client.list_artifacts(1);

    println!(
        "Showing {} of {}",
        artifact_response.artifacts.len(),
        artifact_response.total_count
    );

    let artifact = &artifact_response.artifacts[0];
    println!("{}", serde_json::to_string_pretty(artifact).unwrap());

    let file = client.get_bytes(&artifact.archive_download_url);
    let file = maybe_unzip_single_file(file);
    let file = zstd_decode(&file);
    let file = String::from_utf8(file).expect("Expected report to be valid utf8");

    let report: WptReport = serde_json::from_str(&file).unwrap();

    let mut scores = score_wpt_report(&report);
    scores.retain(|area, _| is_focus_area(area));

    let scores_json = serde_json::to_string_pretty(&scores).unwrap();
    let scores_zstd = zstd_encode(scores_json.as_bytes(), 22);

    println!("{}", scores_json);
    println!("{}", scores_json.as_bytes().len());
    println!("{}", scores_zstd.len());

    let focus_areas = focus_areas(&scores);
    let summary = summarize_results(
        &[RunInfoWithScores {
            date: "2026-04-13".into(),
            info: report.run_info,
            scores: scores,
        }],
        Some(&focus_areas),
    );
    let summary_json = serde_json::to_string_pretty(&summary).unwrap();
    let summary_zstd = zstd_encode(summary_json.as_bytes(), 22);

    println!("{}", summary_json);
    println!("{}", summary_json.as_bytes().len());
    println!("{}", summary_zstd.len());

    // std::fs::write("./scores.json", scores_json.as_bytes()).unwrap();

    // for artifact in &artifact_response.artifacts {
    //     println!("{}", serde_json::to_string_pretty(artifact).unwrap());
    // }
}

fn is_focus_area(area: &str) -> bool {
    let slash_count = area.chars().filter(|c| *c == '/').count();
    slash_count < 2 || (slash_count == 2 && area.starts_with("css/CSS2"))
}

fn focus_areas(scores: &BTreeMap<String, AreaScores>) -> Vec<FocusArea> {
    let mut focus_areas = Vec::new();
    for area in scores.keys() {
        if is_focus_area(area) {
            focus_areas.push(FocusArea {
                name: area.clone(),
                areas: vec![area.clone()],
            });
        }
    }

    focus_areas.sort_unstable_by(|a, b| a.name.cmp(&b.name));

    focus_areas
}
