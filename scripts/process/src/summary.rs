use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use chrono::{DateTime, SecondsFormat};
use serde::{Deserialize, Serialize};
use wptreport::{
    AreaScores, score_wpt_report,
    wpt_report::{TestStatus, WptReport},
};

/// Per-area scores for a single run, stored as a compact
/// `[total_tests, total_score, total_subtests, total_subtests_passed]` array
/// (with `total_score` rounded to 1dp)
pub type ScoreTuple = (u32, f64, u32, u32);

fn score_tuple(scores: &AreaScores) -> ScoreTuple {
    (
        scores.tests.total,
        (scores.servo_score() * 10.0).round() / 10.0,
        scores.subtests.total,
        scores.subtests.pass,
    )
}

/// Metadata about a single WPT run, stored once in `runs.json` and shared by
/// all per-area score files (which are index-aligned with it)
#[derive(Clone, Serialize, Deserialize)]
pub struct RunMeta {
    /// The date of the blitz commit (falling back to when the WPT run was
    /// executed if the commit isn't found), RFC3339 format
    pub date: String,
    /// The revision of the WPT test suite that was run (9-char sha)
    pub wpt_revision: String,
    /// The blitz commit that was tested (full sha)
    pub product_revision: String,
    /// First line of the blitz commit's message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_message: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct RunsFile {
    runs: Vec<RunMeta>,
}

/// Scores for a single area (one WPT folder). `scores` is index-aligned with
/// `runs.json`: one entry per run. `null` marks a run with no data for this
/// area.
#[derive(Serialize, Deserialize)]
pub struct AreaFile {
    pub scores: Vec<Option<ScoreTuple>>,
}

/// A run scored across every directory of the WPT tree
pub struct ScoredRun {
    pub meta: RunMeta,
    pub scores: BTreeMap<String, ScoreTuple>,
}

/// Convert a report into a `ScoredRun`, scoring every directory of the tree.
/// Skipped tests are stripped before scoring (matching the blitz website).
pub fn score_report(
    mut report: WptReport,
    commit_id: &str,
    commit_timestamp: Option<i64>,
    commit_message: Option<String>,
) -> ScoredRun {
    report
        .results
        .retain(|test| test.status != TestStatus::Skip);

    let scores = score_wpt_report::<WptReport>(&report);

    let timestamp = commit_timestamp.unwrap_or(report.time_start as i64);
    let date = DateTime::from_timestamp(timestamp, 0)
        .expect("valid unix timestamp")
        .to_rfc3339_opts(SecondsFormat::Secs, true);

    ScoredRun {
        meta: RunMeta {
            date,
            wpt_revision: report.run_info.revision[0..9].to_string(),
            product_revision: commit_id.to_string(),
            commit_message,
        },
        scores: scores
            .iter()
            .map(|(area, scores)| (area.clone(), score_tuple(scores)))
            .collect(),
    }
}

/// The whole summary dataset: shared run metadata plus one score file per
/// area
#[derive(Default)]
pub struct SummaryStore {
    pub runs: Vec<RunMeta>,
    pub areas: BTreeMap<String, Vec<Option<ScoreTuple>>>,
}

fn load_area_files(
    dir: &Path,
    prefix: &str,
    run_count: usize,
    areas: &mut BTreeMap<String, Vec<Option<ScoreTuple>>>,
) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            let name = path.file_name().unwrap().to_str().unwrap();
            load_area_files(&path, &format!("{prefix}{name}/"), run_count, areas);
        } else if path.extension().is_some_and(|ext| ext == "json") {
            let stem = path.file_stem().unwrap().to_str().unwrap();
            let name = format!("{prefix}{stem}");
            let file: AreaFile = serde_json::from_slice(&std::fs::read(&path).unwrap())
                .expect("area file should be valid JSON");
            assert_eq!(
                file.scores.len(),
                run_count,
                "area file {name} is misaligned with runs.json"
            );
            areas.insert(name, file.scores);
        }
    }
}

impl SummaryStore {
    pub fn load(dir: &Path) -> Option<Self> {
        let runs_file: RunsFile =
            serde_json::from_slice(&std::fs::read(dir.join("runs.json")).ok()?)
                .expect("runs.json should be valid JSON");
        let mut areas = BTreeMap::new();
        load_area_files(&dir.join("areas"), "", runs_file.runs.len(), &mut areas);
        Some(SummaryStore {
            runs: runs_file.runs,
            areas,
        })
    }

    /// Append runs, skipping any whose product revision is already present,
    /// then re-sort all files consistently by (date, product_revision)
    pub fn append(&mut self, new_runs: Vec<ScoredRun>) {
        let existing: HashSet<String> = self
            .runs
            .iter()
            .map(|run| run.product_revision.clone())
            .collect();

        for run in new_runs {
            if existing.contains(&run.meta.product_revision) {
                continue;
            }

            // Ensure every area of this run has a file, padding new areas
            // with nulls for pre-existing runs
            let run_count = self.runs.len();
            for area in run.scores.keys() {
                self.areas
                    .entry(area.clone())
                    .or_insert_with(|| vec![None; run_count]);
            }

            // Append this run's score to every area file (null for areas the
            // run has no data for)
            for (area, scores) in &mut self.areas {
                scores.push(run.scores.get(area).copied());
            }
            self.runs.push(run.meta);
        }

        self.sort();
    }

    /// Sort runs by (date, product_revision), applying the same permutation
    /// to every area file so they stay index-aligned
    fn sort(&mut self) {
        let mut order: Vec<usize> = (0..self.runs.len()).collect();
        order.sort_by(|&a, &b| {
            let ka = (&self.runs[a].date, &self.runs[a].product_revision);
            let kb = (&self.runs[b].date, &self.runs[b].product_revision);
            ka.cmp(&kb)
        });

        self.runs = order.iter().map(|&i| self.runs[i].clone()).collect();
        for scores in self.areas.values_mut() {
            *scores = order.iter().map(|&i| scores[i]).collect();
        }
    }

    /// Write the store to `dir` as runs.json + areas/<area>.json, with one
    /// line per run in each file so appends produce one-line git diffs
    pub fn write(&self, dir: &Path) {
        let areas_dir = dir.join("areas");
        std::fs::create_dir_all(&areas_dir).unwrap();

        write_json_lines(
            &dir.join("runs.json"),
            "{\n\"runs\":[\n",
            self.runs.iter().map(serde_json::to_string),
        );

        for (area, scores) in &self.areas {
            let path = areas_dir.join(format!("{area}.json"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            write_json_lines(
                &path,
                "{\n\"scores\":[\n",
                scores.iter().map(serde_json::to_string),
            );
        }
    }
}

fn write_json_lines(
    path: &Path,
    header: &str,
    lines: impl Iterator<Item = serde_json::Result<String>>,
) {
    let mut json = String::from(header);
    for (i, line) in lines.enumerate() {
        if i > 0 {
            json.push_str(",\n");
        }
        json.push_str(&line.unwrap());
    }
    json.push_str("\n]}\n");
    std::fs::write(path, json).unwrap();
}
