//! Reads run results out of a clone of
//! https://github.com/web-platform-tests/results-analysis-cache
//!
//! Every wpt.fyi run is an orphan commit tagged `run/<id>/results`, whose tree
//! mirrors the WPT directory structure with one JSON blob per test at
//! `<dirs>/<url-encoded test file name>.json`. Blobs are the test's wptreport
//! entry reduced to `status`, `name` and `subtests[].{name,status}`.

use std::path::Path;

use percent_encoding::percent_decode_str;
use serde::Deserialize;
use wptreport::SubtestCounts;

use crate::score::{RunResults, TestCounts};

pub struct Cache {
    repo: gix::Repository,
}

#[derive(Deserialize)]
struct CachedTest {
    status: String,
    #[serde(default)]
    subtests: Vec<CachedSubtest>,
}

#[derive(Deserialize)]
struct CachedSubtest {
    status: String,
}

impl Cache {
    pub fn open(path: &Path) -> Self {
        let repo = gix::open(path)
            .unwrap_or_else(|err| panic!("failed to open {}: {err}", path.display()));
        Self { repo }
    }

    /// Read a run's results, optionally restricted to a subtree (e.g. `css`).
    /// Skipped tests are dropped. Returns `None` if the run isn't cached, or
    /// is cached with an empty tree (which happens for some early runs whose
    /// raw report was unavailable).
    pub fn read_run(&self, run_id: u64, subtree: Option<&str>) -> Option<RunResults> {
        let mut reference = self.repo.find_reference(&tag_name(run_id)).ok()?;
        let mut tree = reference
            .peel_to_tree()
            .expect("run tag should point at a tree");

        let mut prefix = String::new();
        if let Some(subtree) = subtree {
            let entry = tree.peel_to_entry_by_path(subtree).expect("tree lookup")?;
            tree = entry.object().expect("subtree object").into_tree();
            prefix = format!("{}/", subtree.trim_matches('/'));
        }

        let entries = tree
            .traverse()
            .breadthfirst
            .files()
            .expect("tree traversal");

        let mut tests = Vec::with_capacity(entries.len());
        for entry in entries {
            if !entry.mode.is_blob() {
                continue;
            }
            let data = &self.repo.find_object(entry.oid).expect("blob").data;
            let cached: CachedTest = serde_json::from_slice(data)
                .unwrap_or_else(|err| panic!("invalid test blob {}: {err}", entry.filepath));
            if cached.status == "SKIP" {
                continue;
            }
            tests.push(TestCounts {
                path: test_path(&prefix, &entry.filepath),
                passes: cached.status == "PASS",
                subtests: SubtestCounts {
                    pass: cached
                        .subtests
                        .iter()
                        .filter(|subtest| subtest.status == "PASS")
                        .count() as u32,
                    total: cached.subtests.len() as u32,
                },
            });
        }

        if tests.is_empty() {
            return None;
        }
        Some(RunResults { tests })
    }
}

fn tag_name(run_id: u64) -> String {
    format!("refs/tags/run/{run_id}/results")
}

/// Convert a blob path in the cache (`css/css-flexbox/foo.html%3Fa%3Db.json`)
/// back into a WPT test path (`css/css-flexbox/foo.html?a=b`)
fn test_path(prefix: &str, filepath: &[u8]) -> String {
    let filepath = std::str::from_utf8(filepath).expect("utf-8 path");
    let filepath = filepath.strip_suffix(".json").unwrap_or(filepath);
    let (dirs, encoded_name) = filepath.rsplit_once('/').unwrap_or(("", filepath));
    let name = percent_decode_str(encoded_name).decode_utf8_lossy();
    if dirs.is_empty() {
        format!("{prefix}{name}")
    } else {
        format!("{prefix}{dirs}/{name}")
    }
}
