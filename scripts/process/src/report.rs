use std::{collections::HashSet, fs::read_dir, path::Path};

use wptreport::wpt_report::WptReport;

use crate::compression::zstd_decode;

pub fn parse_zstd_report(file: &[u8]) -> Result<WptReport, ()> {
    let file = zstd_decode(&file);
    let file = String::from_utf8(file).map_err(|_| ())?;
    serde_json::from_str(&file).map_err(|_| ())
}

pub fn load_existing_reports(dir: impl AsRef<Path>) -> HashSet<String> {
    let reports_dir = read_dir(dir).unwrap();

    reports_dir
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().unwrap().is_file())
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            if name.ends_with(".json.zst") {
                Some(name.trim_end_matches(".json.zst").to_string())
            } else {
                None
            }
        })
        .collect()
}
