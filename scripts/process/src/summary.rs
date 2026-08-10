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

/// Scores for one group of areas (one top-level WPT folder). `scores` is
/// index-aligned with `runs.json`: one row per run, each row parallel to
/// `focus_areas`. `null` marks an area with no data for that run.
#[derive(Serialize, Deserialize)]
pub struct AreaFile {
    pub focus_areas: Vec<String>,
    pub scores: Vec<Vec<Option<ScoreTuple>>>,
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

/// The name of the group (and so the file, nested under the suite's root
/// directory) that an area's scores are stored in. Areas of depth <= 1 (e.g.
/// "css", "css/css-flexbox") belong to the root group ("css/css"); deeper
/// areas belong to their top-level folder's group. Depth-1 areas additionally
/// head their own group so that each group file contains the full subtree
/// including the folder itself.
fn area_groups(area: &str) -> Vec<String> {
    let mut components = area.split('/');
    let root = components.next().unwrap();
    match components.next() {
        None => vec![format!("{root}/{root}")],
        Some(second) if components.next().is_none() => {
            vec![format!("{root}/{root}"), format!("{root}/{second}")]
        }
        Some(second) => vec![format!("{root}/{second}")],
    }
}

/// The whole summary dataset: shared run metadata plus per-group score files
#[derive(Default)]
pub struct SummaryStore {
    pub runs: Vec<RunMeta>,
    pub areas: BTreeMap<String, AreaFile>,
}

impl SummaryStore {
    pub fn load(dir: &Path) -> Option<Self> {
        let runs_file: RunsFile =
            serde_json::from_slice(&std::fs::read(dir.join("runs.json")).ok()?)
                .expect("runs.json should be valid JSON");
        let mut areas = BTreeMap::new();
        for root_entry in std::fs::read_dir(dir.join("areas")).expect("summary/areas should exist")
        {
            let root_path = root_entry.unwrap().path();
            let root = root_path.file_name().unwrap().to_str().unwrap().to_string();
            for entry in std::fs::read_dir(&root_path).unwrap() {
                let path = entry.unwrap().path();
                let stem = path.file_stem().unwrap().to_str().unwrap();
                let name = format!("{root}/{stem}");
                let file: AreaFile = serde_json::from_slice(&std::fs::read(&path).unwrap())
                    .expect("area file should be valid JSON");
                assert_eq!(
                    file.scores.len(),
                    runs_file.runs.len(),
                    "area file {name} is misaligned with runs.json"
                );
                areas.insert(name, file);
            }
        }
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

            // Group this run's areas
            let mut grouped: BTreeMap<String, BTreeMap<&str, ScoreTuple>> = BTreeMap::new();
            for (area, scores) in &run.scores {
                for group in area_groups(area) {
                    grouped.entry(group).or_default().insert(area, *scores);
                }
            }

            // Ensure every group file exists and contains every area of this
            // run, padding pre-existing rows with nulls for new columns
            let run_count = self.runs.len();
            for (group, area_scores) in &grouped {
                let file = self.areas.entry(group.clone()).or_insert_with(|| AreaFile {
                    focus_areas: Vec::new(),
                    scores: vec![Vec::new(); run_count],
                });
                for area in area_scores.keys() {
                    if !file.focus_areas.iter().any(|a| a == area) {
                        file.focus_areas.push(area.to_string());
                        for row in &mut file.scores {
                            row.push(None);
                        }
                    }
                }
            }

            // Append this run's row to every group file (null-filled for
            // groups the run has no data for)
            for (group, file) in &mut self.areas {
                let area_scores = grouped.get(group);
                let row = file
                    .focus_areas
                    .iter()
                    .map(|area| area_scores.and_then(|scores| scores.get(area.as_str()).copied()))
                    .collect();
                file.scores.push(row);
            }
            self.runs.push(run.meta);
        }

        self.sort();
    }

    /// Sort runs by (date, product_revision), applying the same permutation
    /// to every area file so they stay index-aligned. Also sorts each file's
    /// area columns alphabetically.
    fn sort(&mut self) {
        let mut order: Vec<usize> = (0..self.runs.len()).collect();
        order.sort_by(|&a, &b| {
            let ka = (&self.runs[a].date, &self.runs[a].product_revision);
            let kb = (&self.runs[b].date, &self.runs[b].product_revision);
            ka.cmp(&kb)
        });

        self.runs = order.iter().map(|&i| self.runs[i].clone()).collect();
        for file in self.areas.values_mut() {
            let mut columns: Vec<usize> = (0..file.focus_areas.len()).collect();
            columns.sort_by(|&a, &b| file.focus_areas[a].cmp(&file.focus_areas[b]));
            file.focus_areas = columns
                .iter()
                .map(|&c| file.focus_areas[c].clone())
                .collect();
            file.scores = order
                .iter()
                .map(|&i| columns.iter().map(|&c| file.scores[i][c]).collect())
                .collect();
        }
    }

    /// Write the store to `dir` as runs.json + areas/<group>.json, with one
    /// line per run in each file so appends produce one-line git diffs
    pub fn write(&self, dir: &Path) {
        let areas_dir = dir.join("areas");
        std::fs::create_dir_all(&areas_dir).unwrap();

        write_json_lines(
            &dir.join("runs.json"),
            "{\n\"runs\":[\n",
            self.runs.iter().map(serde_json::to_string),
        );

        for (group, file) in &self.areas {
            let path = areas_dir.join(format!("{group}.json"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            write_json_lines(
                &path,
                &format!(
                    "{{\n\"focus_areas\":{},\n\"scores\":[\n",
                    serde_json::to_string(&file.focus_areas).unwrap()
                ),
                file.scores.iter().map(serde_json::to_string),
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
