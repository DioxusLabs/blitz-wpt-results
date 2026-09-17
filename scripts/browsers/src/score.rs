//! Scoring of browser runs into the `summary/` format.
//!
//! Both data sources (results-analysis-cache blobs and wpt.fyi summary files)
//! are reduced to a [`TestCounts`] per test, which is all `wptreport`'s scoring
//! needs. A test with no subtests counts as a single subtest that passes iff
//! the test status is `PASS`, matching `wptreport::wpt_report::TestResult`.

use std::collections::BTreeMap;

use chrono::SecondsFormat;
use process::summary::{RunMeta, ScoreTuple, ScoredRun, score_tuple};
use wptreport::{
    ScorableReport, SubtestCounts, SubtestNameAndResult, TestResultIter, score_wpt_report,
};

use crate::wptfyi::Run;

/// Per-test data needed for scoring
pub struct TestCounts {
    /// Test path without leading slash, e.g. `css/css-flexbox/foo.html?variant`
    /// (matching the `test` names in blitz's wptreports)
    pub path: String,
    /// Whether the test-level status is `PASS`
    pub passes: bool,
    /// `[passed, total]` subtests. Both zero for tests without subtests.
    pub subtests: SubtestCounts,
}

pub struct RunResults {
    pub tests: Vec<TestCounts>,
}

impl TestResultIter for &TestCounts {
    fn name(&self) -> &str {
        &self.path
    }

    fn subtest_counts(&self) -> SubtestCounts {
        if self.subtests.total == 0 {
            SubtestCounts {
                total: 1,
                pass: self.passes as u32,
            }
        } else {
            self.subtests
        }
    }

    fn subtest_exist_and_passes(&self, _name: &str) -> bool {
        false
    }

    fn iter_subtests_results(&self) -> impl Iterator<Item = SubtestNameAndResult<'_>> {
        std::iter::empty()
    }
}

#[rustfmt::skip]
impl ScorableReport for RunResults {
    type TestResultIter<'a> = &'a TestCounts where Self: 'a;
    fn results(&self) -> impl Iterator<Item = Self::TestResultIter<'_>> {
        // `score_wpt_report` requires every test to live in a directory
        self.tests.iter().filter(|test| test.path.contains('/'))
    }
}

/// Score a run across every directory of the WPT tree. Skipped tests must
/// already have been stripped by the data source.
pub fn score_run(run: &Run, results: &RunResults) -> ScoredRun {
    let scores: BTreeMap<String, ScoreTuple> = score_wpt_report(results)
        .iter()
        .map(|(area, scores)| (area.clone(), score_tuple(scores)))
        .collect();

    let date = run.start().to_rfc3339_opts(SecondsFormat::Secs, true);

    ScoredRun {
        meta: RunMeta {
            date,
            wpt_revision: run.full_revision_hash[..9].to_string(),
            product_revision: run.browser_version.clone(),
            commit_message: None,
            run_id: Some(run.id),
        },
        scores,
    }
}
