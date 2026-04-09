use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::layout::{LOG_DIR, format_log_version};

/// Metadata recorded for every Parquet file added to the table.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddFile {
    /// Path relative to the table root (e.g. "data/01KNQ....parquet")
    pub path: String,
    /// File size in bytes, recorded at write time
    pub size: u64,
    /// Number of rows in the file
    pub row_count: u64,
    /// Partition column → value map; empty until partitioning is introduced
    pub partition_values: HashMap<String, String>,
}

/// A single action written to a log file.
///
/// Each log file is newline-delimited JSON (NDJSON): one action per line.
/// The enum variant name becomes the JSON key — `{"add": {...}}` — matching
/// the Delta Lake log format so the concepts transfer directly.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    Add(AddFile),
}

/// Write `actions` to `<table_dir>/_log/<version>.json`.
///
/// Each action is serialized as one JSON line (NDJSON).
/// The log directory is created if it does not exist.
/// This is a direct write — atomic rename comes in W2-2.
pub fn commit(table_dir: &Path, version: u64, actions: &[Action]) -> anyhow::Result<()> {
    let log_dir = table_dir.join(LOG_DIR);
    fs::create_dir_all(&log_dir)
        .with_context(|| format!("failed to create log dir: {}", log_dir.display()))?;

    let filename = format!("{}.json", format_log_version(version));
    let log_path = log_dir.join(&filename);

    let mut file = fs::File::create(&log_path)
        .with_context(|| format!("failed to create log file: {}", log_path.display()))?;

    for action in actions {
        let line = serde_json::to_string(action)
            .context("failed to serialize action")?;
        writeln!(file, "{}", line).context("failed to write action line")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_creates_log_file_with_parseable_json() {
        let dir = tempfile::tempdir().expect("temp dir");

        let add = AddFile {
            path: "data/01TESTULID.parquet".to_string(),
            size: 4096,
            row_count: 3,
            partition_values: HashMap::new(),
        };

        commit(dir.path(), 0, &[Action::Add(add)]).expect("commit");

        // Log file must exist at the expected path
        let log_path = dir.path().join("_log/00000000000000000000.json");
        assert!(log_path.exists(), "log file not found at expected path");

        // Every line must be valid JSON and contain an "add" key
        let contents = fs::read_to_string(&log_path).expect("read log file");
        for line in contents.lines() {
            let v: serde_json::Value =
                serde_json::from_str(line).expect("line is not valid JSON");
            assert!(v.get("add").is_some(), "expected 'add' key in: {}", line);
        }
    }

    #[test]
    fn commit_records_correct_metadata() {
        let dir = tempfile::tempdir().expect("temp dir");

        let add = AddFile {
            path: "data/myfile.parquet".to_string(),
            size: 8192,
            row_count: 100,
            partition_values: HashMap::new(),
        };

        commit(dir.path(), 1, &[Action::Add(add)]).expect("commit");

        let log_path = dir.path().join("_log/00000000000000000001.json");
        let contents = fs::read_to_string(&log_path).expect("read log file");
        let v: serde_json::Value = serde_json::from_str(contents.trim()).expect("valid JSON");

        assert_eq!(v["add"]["path"], "data/myfile.parquet");
        assert_eq!(v["add"]["size"], 8192);
        assert_eq!(v["add"]["rowCount"], 100);
    }
}
