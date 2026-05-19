use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::layout::{format_log_version, LOG_DIR};

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

/// Marks a previously-added file as logically deleted.
///
/// After a `RemoveFile` is committed, the file is excluded from any snapshot
/// at or after that version.  The physical file in `data/` is untouched —
/// time-travel queries at earlier versions still reference it.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveFile {
    /// Same relative path that was recorded in the matching `AddFile`
    pub path: String,
    /// Partition values of the file being removed (mirrors AddFile for consistency)
    pub partition_values: HashMap<String, String>,
}

/// Per-commit housekeeping metadata.
///
/// Always written as the *first* action in every log file so readers can
/// extract the commit timestamp without scanning data actions.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitInfo {
    /// RFC 3339 wall-clock time of the commit (UTC)
    pub timestamp: String,
    /// Human-readable label for what the commit does (e.g. "write", "delete")
    pub operation: String,
    /// Caller-supplied idempotency token; used for exactly-once semantics (W5)
    pub txn_id: Option<String>,
    /// Identifying tag for the application that issued the commit
    pub app_id: Option<String>,
}

/// A single action written to a log file.
///
/// Each log file is newline-delimited JSON (NDJSON): one action per line.
/// The enum variant name becomes the JSON key — `{"add":{...}}` — matching
/// the Delta Lake log format so the concepts transfer directly.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    Add(AddFile),
    Remove(RemoveFile),
    CommitInfo(CommitInfo),
}

/// Options forwarded from the caller to shape the auto-generated `CommitInfo`.
pub struct CommitOptions {
    /// Short label for the operation (default: `"write"`)
    pub operation: String,
    pub txn_id: Option<String>,
    pub app_id: Option<String>,
}

impl Default for CommitOptions {
    fn default() -> Self {
        CommitOptions {
            operation: "write".to_string(),
            txn_id: None,
            app_id: None,
        }
    }
}

/// Write `actions` to `<table_dir>/_log/<version>.json` atomically.
///
/// Atomicity strategy (W2-2): content is first written to a sibling
/// `<version>.json.tmp` file; once all bytes are flushed the file is renamed
/// into place.  On POSIX, `rename(2)` is atomic — readers either see the
/// complete log file or nothing.  A crash before the rename leaves only the
/// `.tmp` file, which readers ignore, so the table remains readable at the
/// previous version.
///
/// A `CommitInfo` action (W2-3) is prepended automatically so every log file
/// carries a wall-clock timestamp and operation label.
pub fn commit(
    table_dir: &Path,
    version: u64,
    actions: &[Action],
    opts: CommitOptions,
) -> anyhow::Result<()> {
    let log_dir = table_dir.join(LOG_DIR);
    fs::create_dir_all(&log_dir)
        .with_context(|| format!("failed to create log dir: {}", log_dir.display()))?;

    let stem = format_log_version(version);
    let log_path = log_dir.join(format!("{stem}.json"));
    let tmp_path = log_dir.join(format!("{stem}.json.tmp"));

    let mut file = fs::File::create(&tmp_path)
        .with_context(|| format!("failed to create tmp log file: {}", tmp_path.display()))?;

    // First line: CommitInfo so readers can find the timestamp without scanning data actions
    let ci = CommitInfo {
        timestamp: chrono::Utc::now().to_rfc3339(),
        operation: opts.operation,
        txn_id: opts.txn_id,
        app_id: opts.app_id,
    };
    let ci_line = serde_json::to_string(&Action::CommitInfo(ci)).context("failed to serialize CommitInfo")?;
    writeln!(file, "{ci_line}").context("failed to write CommitInfo")?;

    for action in actions {
        let line = serde_json::to_string(action).context("failed to serialize action")?;
        writeln!(file, "{line}").context("failed to write action line")?;
    }

    file.flush().context("failed to flush tmp log file")?;
    drop(file);

    // Atomic promotion: rename is a single syscall on POSIX
    fs::rename(&tmp_path, &log_path)
        .with_context(|| format!("failed to rename tmp log to final path: {}", log_path.display()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_add(path: &str) -> Action {
        Action::Add(AddFile {
            path: path.to_string(),
            size: 4096,
            row_count: 3,
            partition_values: HashMap::new(),
        })
    }

    #[test]
    fn commit_creates_log_file_with_parseable_json() {
        let dir = tempfile::tempdir().expect("temp dir");

        commit(dir.path(), 0, &[default_add("data/01TESTULID.parquet")], CommitOptions::default())
            .expect("commit");

        let log_path = dir.path().join("_log/00000000000000000000.json");
        assert!(log_path.exists(), "log file not found at expected path");

        let contents = fs::read_to_string(&log_path).expect("read log file");
        for line in contents.lines() {
            let v: serde_json::Value = serde_json::from_str(line).expect("line is not valid JSON");
            // Each line must be one of the known action keys
            let has_known_key = v.get("add").is_some()
                || v.get("remove").is_some()
                || v.get("commitInfo").is_some();
            assert!(has_known_key, "unexpected JSON structure: {line}");
        }
    }

    #[test]
    fn commit_records_correct_add_metadata() {
        let dir = tempfile::tempdir().expect("temp dir");

        let add = AddFile {
            path: "data/myfile.parquet".to_string(),
            size: 8192,
            row_count: 100,
            partition_values: HashMap::new(),
        };

        commit(dir.path(), 1, &[Action::Add(add)], CommitOptions::default()).expect("commit");

        let log_path = dir.path().join("_log/00000000000000000001.json");
        let contents = fs::read_to_string(&log_path).expect("read log file");

        // Second line is the AddFile action (first is CommitInfo)
        let add_line = contents.lines().nth(1).expect("second line");
        let v: serde_json::Value = serde_json::from_str(add_line).expect("valid JSON");

        assert_eq!(v["add"]["path"], "data/myfile.parquet");
        assert_eq!(v["add"]["size"], 8192);
        assert_eq!(v["add"]["rowCount"], 100);
    }

    #[test]
    fn commit_info_is_first_line_and_has_timestamp() {
        let dir = tempfile::tempdir().expect("temp dir");

        commit(
            dir.path(),
            0,
            &[default_add("data/f.parquet")],
            CommitOptions {
                operation: "test-op".to_string(),
                txn_id: Some("txn-123".to_string()),
                app_id: Some("test-app".to_string()),
            },
        )
        .expect("commit");

        let contents = fs::read_to_string(dir.path().join("_log/00000000000000000000.json")).unwrap();
        let first_line = contents.lines().next().expect("first line");
        let v: serde_json::Value = serde_json::from_str(first_line).unwrap();

        assert!(v.get("commitInfo").is_some(), "first line must be commitInfo");
        assert!(v["commitInfo"]["timestamp"].is_string(), "timestamp must be a string");
        assert_eq!(v["commitInfo"]["operation"], "test-op");
        assert_eq!(v["commitInfo"]["txnId"], "txn-123");
        assert_eq!(v["commitInfo"]["appId"], "test-app");
    }

    // W2-2: partial write simulation — a leftover .tmp file must not corrupt reads
    #[test]
    fn stale_tmp_file_does_not_affect_previous_version() {
        let dir = tempfile::tempdir().expect("temp dir");

        // Commit version 0 normally
        commit(dir.path(), 0, &[default_add("data/file0.parquet")], CommitOptions::default())
            .expect("commit v0");

        // Simulate a crash mid-commit for version 1: write .tmp but never rename
        let log_dir = dir.path().join("_log");
        let tmp_path = log_dir.join("00000000000000000001.json.tmp");
        fs::write(&tmp_path, b"incomplete garbage\n").expect("write orphaned tmp");

        // Version 0 must still be fully readable despite the orphaned .tmp
        let snap = crate::snapshot::read(dir.path(), 0).expect("should read version 0 cleanly");
        assert_eq!(snap.files.len(), 1);
        assert_eq!(snap.files[0].path, "data/file0.parquet");
    }

    // W2-1: RemoveFile round-trip in the log format
    #[test]
    fn remove_action_serializes_correctly() {
        let dir = tempfile::tempdir().expect("temp dir");

        let remove = Action::Remove(RemoveFile {
            path: "data/old.parquet".to_string(),
            partition_values: HashMap::new(),
        });

        commit(dir.path(), 0, &[remove], CommitOptions::default()).expect("commit remove");

        let contents = fs::read_to_string(dir.path().join("_log/00000000000000000000.json")).unwrap();
        let remove_line = contents.lines().nth(1).expect("second line");
        let v: serde_json::Value = serde_json::from_str(remove_line).unwrap();
        assert!(v.get("remove").is_some(), "expected 'remove' key");
        assert_eq!(v["remove"]["path"], "data/old.parquet");
    }
}
