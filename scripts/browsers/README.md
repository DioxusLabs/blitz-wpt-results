# Browser WPT score summaries

Builds `summary/`-format score data (see `summary/README.md`) for browsers
tested on [wpt.fyi](https://wpt.fyi), one dataset per product at
`<out>/<product>/{runs.json,areas/**.json}`.

Two data sources, both derived from the same wptreports so they score
identically:

- **results-analysis-cache** (historical backfill): a local clone of
  [web-platform-tests/results-analysis-cache](https://github.com/web-platform-tests/results-analysis-cache),
  where every wpt.fyi run is an orphan commit tagged `run/<id>/results` with one
  JSON blob per test. One ~2 GB clone covers every run since mid-2017, read with
  `gix` at ~1s per run.
- **wpt.fyi summary files** (incremental): the run's `summary_v2.json.gz`
  (~1 MB), fetched only for runs the cache doesn't have yet (the cache is
  updated every 3 hours). Pre-July-2022 summaries are in the v1 format, which
  folds the harness status into the subtest counts, so prefer the cache for
  anything historical.

```sh
git clone --bare https://github.com/web-platform-tests/results-analysis-cache.git ../results-analysis-cache.git

# Backfill, one run per browser per day
cargo run -rp browsers -- --cache ../results-analysis-cache.git --out browsers --daily

# Incremental update: only runs not already in browsers/<product>/runs.json are
# scored; runs missing from the cache fall back to wpt.fyi summaries
cargo run -rp browsers -- --cache ../results-analysis-cache.git --out browsers --daily --from 2026-09-01

# Options
cargo run -rp browsers -- --help
```

Defaults: products `chrome firefox safari servo`, runs labelled
`master,experimental`. Tests with status `SKIP` are dropped before scoring, as
for Blitz. Runs are de-duplicated by wpt.fyi run ID.
