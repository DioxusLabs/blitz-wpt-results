//! Builds `summary/`-format score data for browsers from wpt.fyi runs.
//!
//! Historical runs are read from a local clone of results-analysis-cache
//! (one bulk clone instead of one download per run); runs not yet in the cache
//! fall back to fetching the run's summary file from wpt.fyi.
//!
//! Output is `<out>/<product>/{runs.json,areas/**.json}`, one dataset per
//! product, in the same format as `summary/` (see `summary/README.md`).

mod cache;
mod score;
mod wptfyi;

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

use process::summary::{ScoredRun, SummaryStore};

use crate::cache::Cache;
use crate::score::score_run;
use crate::wptfyi::{Run, RunQuery};

#[derive(Clone, Copy, PartialEq)]
enum Source {
    /// Cache when available, otherwise wpt.fyi summary files
    Auto,
    /// results-analysis-cache only; skip runs that aren't cached
    Cache,
    /// wpt.fyi summary files only
    Summary,
}

struct Args {
    /// Path to a (bare or regular) clone of results-analysis-cache
    cache: Option<PathBuf>,
    /// Output directory; each product is written to `<out>/<product>`
    out: PathBuf,
    products: Vec<String>,
    /// Labels every run must have (wpt.fyi `labels=` param)
    labels: Vec<String>,
    from: Option<String>,
    to: Option<String>,
    /// Keep at most one run per product per UTC day (the earliest)
    daily: bool,
    /// Stop after this many runs per product
    max_runs: Option<usize>,
    /// Only score this subtree of WPT (e.g. `css`)
    subtree: Option<String>,
    source: Source,
}

const USAGE: &str = "\
Usage: browsers [options]

  --cache <path>       Clone of web-platform-tests/results-analysis-cache
  --out <dir>          Output directory (default: ./browsers)
  --product <name>     wpt.fyi product, repeatable (default: chrome firefox safari servo)
  --labels <a,b>       Required run labels (default: master,experimental)
  --from <date>        Only runs starting after this RFC 3339 / YYYY-MM-DD date
  --to <date>          Only runs starting before this date
  --daily              Keep only the first run per product per UTC day
  --max-runs <n>       Process at most n runs per product
  --subtree <dir>      Only score this WPT directory (e.g. css)
  --source <mode>      auto (default) | cache | summary
";

fn parse_args() -> Args {
    let mut args = Args {
        cache: None,
        out: PathBuf::from("browsers"),
        products: Vec::new(),
        labels: Vec::new(),
        from: None,
        to: None,
        daily: false,
        max_runs: None,
        subtree: None,
        source: Source::Auto,
    };

    let mut iter = std::env::args().skip(1);
    while let Some(flag) = iter.next() {
        let mut value = || {
            iter.next()
                .unwrap_or_else(|| panic!("{flag} requires a value\n\n{USAGE}"))
        };
        match flag.as_str() {
            "--cache" => args.cache = Some(PathBuf::from(value())),
            "--out" => args.out = PathBuf::from(value()),
            "--product" => args.products.push(value()),
            "--labels" => args.labels = value().split(',').map(str::to_owned).collect(),
            "--from" => args.from = Some(normalize_date(&value())),
            "--to" => args.to = Some(normalize_date(&value())),
            "--daily" => args.daily = true,
            "--max-runs" => args.max_runs = Some(value().parse().expect("--max-runs number")),
            "--subtree" => args.subtree = Some(value()),
            "--source" => {
                args.source = match value().as_str() {
                    "auto" => Source::Auto,
                    "cache" => Source::Cache,
                    "summary" => Source::Summary,
                    other => panic!("unknown --source {other}\n\n{USAGE}"),
                }
            }
            "--help" | "-h" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            other => panic!("unknown argument {other}\n\n{USAGE}"),
        }
    }

    if args.products.is_empty() {
        args.products = ["chrome", "firefox", "safari", "servo"]
            .map(str::to_owned)
            .to_vec();
    }
    if args.labels.is_empty() {
        args.labels = ["master", "experimental"].map(str::to_owned).to_vec();
    }
    if args.source != Source::Summary && args.cache.is_none() {
        panic!("--cache is required unless --source summary\n\n{USAGE}");
    }
    args
}

/// Accept bare `YYYY-MM-DD` dates as well as RFC 3339 timestamps
fn normalize_date(date: &str) -> String {
    if date.len() == 10 {
        format!("{date}T00:00:00Z")
    } else {
        date.to_owned()
    }
}

/// Keep the earliest run of each UTC day
fn thin_to_daily(runs: &mut Vec<Run>) {
    runs.sort_by_key(Run::start);
    let mut seen_days = HashSet::new();
    runs.retain(|run| seen_days.insert(run.start().date_naive()));
}

fn main() {
    let args = parse_args();
    let cache = args.cache.as_deref().map(Cache::open);

    for product in &args.products {
        let started = Instant::now();
        let out_dir = args.out.join(product);
        let mut store = SummaryStore::load(&out_dir).unwrap_or_default();
        let existing: HashSet<u64> = store.runs.iter().filter_map(|run| run.run_id).collect();

        let mut runs = wptfyi::list_runs(&RunQuery {
            product,
            labels: &args.labels,
            from: args.from.as_deref(),
            to: args.to.as_deref(),
        });
        println!("{product}: {} runs match on wpt.fyi", runs.len());

        if args.daily {
            thin_to_daily(&mut runs);
        }
        runs.retain(|run| !existing.contains(&run.id));
        if let Some(max) = args.max_runs {
            runs.truncate(max);
        }

        let mut scored: Vec<ScoredRun> = Vec::with_capacity(runs.len());
        let (mut from_cache, mut from_summary, mut skipped) = (0, 0, 0);
        for (idx, run) in runs.iter().enumerate() {
            let cached = match (&cache, args.source) {
                (Some(cache), Source::Auto | Source::Cache) => {
                    cache.read_run(run.id, args.subtree.as_deref())
                }
                _ => None,
            };
            let results = match cached {
                Some(results) => {
                    from_cache += 1;
                    results
                }
                None if args.source == Source::Cache => {
                    skipped += 1;
                    continue;
                }
                None => {
                    from_summary += 1;
                    let mut results = wptfyi::fetch_summary(run);
                    if let Some(subtree) = &args.subtree {
                        let prefix = format!("{}/", subtree.trim_matches('/'));
                        results.tests.retain(|test| test.path.starts_with(&prefix));
                    }
                    results
                }
            };
            scored.push(score_run(run, &results));
            if (idx + 1) % 50 == 0 {
                println!("{product}: scored {}/{} runs", idx + 1, runs.len());
            }
        }

        store.append(scored);
        store.write(&out_dir);
        println!(
            "{product}: wrote {} runs across {} areas to {} ({from_cache} from cache, \
             {from_summary} from wpt.fyi summaries, {skipped} skipped) in {:.1?}",
            store.runs.len(),
            store.areas.len(),
            out_dir.display(),
            started.elapsed()
        );
    }
}
