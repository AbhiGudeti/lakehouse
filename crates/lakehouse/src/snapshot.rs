use std::fs;
use std::path::Path;

use anyhow::Context;
use chrono::{DateTime, FixedOffset};

use crate::layout::{format_log_version, LOG_DIR};
use crate::log::{Action, AddFile};

/// The reconstructed state of a table at a specific version.
///
/// Derived by replaying the transaction log; never stored on disk.
/// Will grow to include schema (W3-1) and partition info.
#[derive(Debug)]
pub struct Snapshot {
    pub version: u64,
    pub files: Vec<AddFile>,
}

/// Replay the transaction log from version 0 up to and including `version`,
/// returning the set of active files at that point in time.
///
/// - `Action::Add`        → file enters the active set
/// - `Action::Remove`     → file is evicted from the active set (W2-1)
/// - `Action::CommitInfo` → metadata only; does not affect the file set (W2-3)
///
/// Unknown JSON fields within any action are silently ignored by serde, so
/// log files written by a newer version of this code are forward-compatible.
pub fn read(table_dir: &Path, version: u64) -> anyhow::Result<Snapshot> {
    let log_dir = table_dir.join(LOG_DIR);
    let mut files: Vec<AddFile> = Vec::new();

    for v in 0..=version {
        let filename = format!("{}.json", format_log_version(v));
        let log_path = log_dir.join(&filename);

        let contents = fs::read_to_string(&log_path)
            .with_context(|| format!("log file missing for version {v}: {}", log_path.display()))?;

        for line in contents.lines() {
            let action: Action = serde_json::from_str(line)
                .with_context(|| format!("invalid JSON in {filename}: {line}"))?;

            match action {
                Action::Add(add_file) => files.push(add_file),
                Action::Remove(remove_file) => {
                    // Evict the matching file from the active set.
                    // Path comparison is exact — the same relative string written by AddFile.
                    files.retain(|f| f.path != remove_file.path);
                }
                Action::CommitInfo(_) => {}
            }
        }
    }

    Ok(Snapshot { version, files })
}

/// Scan `_log/` to find the highest committed version number.
///
/// Returns `None` if no log files exist (table is empty / uninitialized).
/// Only `.json` files with exactly 20-digit stems are counted — `.tmp` files
/// left by a crashed commit are invisible to this scan.
pub fn latest_version(table_dir: &Path) -> anyhow::Result<Option<u64>> {
    let log_dir = table_dir.join(LOG_DIR);

    if !log_dir.exists() {
        return Ok(None);
    }

    let mut max: Option<u64> = None;

    for entry in fs::read_dir(&log_dir)
        .with_context(|| format!("failed to read log dir: {}", log_dir.display()))?
    {
        let entry = entry.context("failed to read dir entry")?;
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if let Some(stem) = name.strip_suffix(".json") {
            if stem.len() == 20 {
                if let Ok(v) = stem.parse::<u64>() {
                    max = Some(max.map_or(v, |prev| prev.max(v)));
                }
            }
        }
    }

    Ok(max)
}

/// Return the version number to use for the next commit.
///
/// If no commits exist yet, returns 0.
pub fn next_version(table_dir: &Path) -> anyhow::Result<u64> {
    Ok(latest_version(table_dir)?.map_or(0, |v| v + 1))
}

/// Read the `CommitInfo` timestamp from a specific log version.
///
/// The CommitInfo is always the first NDJSON line (by our write convention),
/// but this function scans all lines to be safe.
fn read_commit_timestamp(table_dir: &Path, version: u64) -> anyhow::Result<String> {
    let log_dir = table_dir.join(LOG_DIR);
    let filename = format!("{}.json", format_log_version(version));
    let log_path = log_dir.join(&filename);

    let contents = fs::read_to_string(&log_path)
        .with_context(|| format!("cannot open log version {version} for timestamp read"))?;

    for line in contents.lines() {
        let action: Action = serde_json::from_str(line)
            .with_context(|| format!("invalid JSON in {filename} while reading timestamp"))?;
        if let Action::CommitInfo(ci) = action {
            return Ok(ci.timestamp);
        }
    }

    anyhow::bail!("no CommitInfo found in log version {version} — was this log written before W2-3?")
}

/// Resolve an RFC 3339 timestamp to a snapshot version.
///
/// Scans all committed versions in order and returns the **latest** version
/// whose `CommitInfo.timestamp` is ≤ `timestamp`.  This mirrors how Delta
/// Lake implements `AS OF TIMESTAMP`: the snapshot you would have seen had
/// you queried at that moment.
///
/// Errors if no version falls at or before the requested timestamp.
pub fn version_at_timestamp(table_dir: &Path, timestamp: &str) -> anyhow::Result<u64> {
    let requested: DateTime<FixedOffset> = DateTime::parse_from_rfc3339(timestamp)
        .with_context(|| format!("invalid timestamp '{timestamp}' — use RFC 3339, e.g. 2026-03-01T12:00:00Z"))?;

    let latest = latest_version(table_dir)?.context("table has no committed versions")?;

    let mut result: Option<u64> = None;

    for v in 0..=latest {
        let ts = read_commit_timestamp(table_dir, v)?;
        let commit_dt: DateTime<FixedOffset> = DateTime::parse_from_rfc3339(&ts)
            .with_context(|| format!("log version {v} has malformed timestamp: {ts}"))?;

        if commit_dt <= requested {
            result = Some(v);
        }
        // Versions are committed in monotonically increasing wall-clock order,
        // so once we overshoot the requested timestamp we can stop early.
        // (We don't break here to be safe against clock skew in future multi-writer scenarios.)
    }

    result.with_context(|| {
        format!("no committed version exists at or before '{timestamp}'")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::{self, Action, AddFile, CommitOptions, RemoveFile};
    use std::collections::HashMap;

    fn make_add(path: &str) -> Action {
        Action::Add(AddFile {
            path: path.to_string(),
            size: 1024,
            row_count: 10,
            partition_values: HashMap::new(),
        })
    }

    fn make_remove(path: &str) -> Action {
        Action::Remove(RemoveFile {
            path: path.to_string(),
            partition_values: HashMap::new(),
        })
    }

    #[test]
    fn snapshot_collects_all_add_files() {
        let dir = tempfile::tempdir().expect("temp dir");

        log::commit(dir.path(), 0, &[make_add("data/file0.parquet")], CommitOptions::default()).unwrap();
        log::commit(dir.path(), 1, &[make_add("data/file1.parquet")], CommitOptions::default()).unwrap();

        let snap = read(dir.path(), 1).expect("read snapshot");

        assert_eq!(snap.version, 1);
        assert_eq!(snap.files.len(), 2);
        assert_eq!(snap.files[0].path, "data/file0.parquet");
        assert_eq!(snap.files[1].path, "data/file1.parquet");
    }

    #[test]
    fn snapshot_at_version_0_has_only_first_file() {
        let dir = tempfile::tempdir().expect("temp dir");

        log::commit(dir.path(), 0, &[make_add("data/file0.parquet")], CommitOptions::default()).unwrap();
        log::commit(dir.path(), 1, &[make_add("data/file1.parquet")], CommitOptions::default()).unwrap();

        let snap = read(dir.path(), 0).expect("read snapshot");

        assert_eq!(snap.files.len(), 1);
        assert_eq!(snap.files[0].path, "data/file0.parquet");
    }

    #[test]
    fn next_version_is_zero_for_empty_table() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert_eq!(next_version(dir.path()).unwrap(), 0);
    }

    #[test]
    fn next_version_increments_after_commit() {
        let dir = tempfile::tempdir().expect("temp dir");

        log::commit(dir.path(), 0, &[make_add("data/file0.parquet")], CommitOptions::default()).unwrap();
        assert_eq!(next_version(dir.path()).unwrap(), 1);

        log::commit(dir.path(), 1, &[make_add("data/file1.parquet")], CommitOptions::default()).unwrap();
        assert_eq!(next_version(dir.path()).unwrap(), 2);
    }

    // W2-1: RemoveFile evicts the file from the active snapshot
    #[test]
    fn remove_file_evicts_from_snapshot() {
        let dir = tempfile::tempdir().expect("temp dir");

        log::commit(dir.path(), 0, &[make_add("data/a.parquet")], CommitOptions::default()).unwrap();
        log::commit(dir.path(), 1, &[make_remove("data/a.parquet")], CommitOptions::default()).unwrap();

        let snap = read(dir.path(), 1).expect("snapshot after remove");
        assert!(snap.files.is_empty(), "file A should no longer appear after RemoveFile");
    }

    // W2-1: snapshot at version before the remove still sees the file
    #[test]
    fn snapshot_before_remove_still_has_file() {
        let dir = tempfile::tempdir().expect("temp dir");

        log::commit(dir.path(), 0, &[make_add("data/a.parquet")], CommitOptions::default()).unwrap();
        log::commit(dir.path(), 1, &[make_remove("data/a.parquet")], CommitOptions::default()).unwrap();

        let snap = read(dir.path(), 0).expect("snapshot at v0");
        assert_eq!(snap.files.len(), 1, "file A must still be visible at version 0");
    }

    // W2-4: timestamp-based time travel resolves to correct version
    #[test]
    fn version_at_timestamp_resolves_correctly() {
        use std::time::Duration;

        let dir = tempfile::tempdir().expect("temp dir");

        // Commit v0, record time, sleep briefly, commit v1, record time.
        // Then assert timestamp between commits resolves to v0.
        log::commit(dir.path(), 0, &[make_add("data/v0.parquet")], CommitOptions::default()).unwrap();

        // Small sleep so the two commits have distinct timestamps
        std::thread::sleep(Duration::from_millis(20));
        let between = chrono::Utc::now().to_rfc3339();
        std::thread::sleep(Duration::from_millis(20));

        log::commit(dir.path(), 1, &[make_add("data/v1.parquet")], CommitOptions::default()).unwrap();

        let resolved = version_at_timestamp(dir.path(), &between).expect("resolve timestamp");
        assert_eq!(resolved, 0, "timestamp between v0 and v1 must resolve to version 0");
    }

    // W2-4: timestamp after all commits resolves to latest
    #[test]
    fn version_at_timestamp_after_all_commits_gives_latest() {
        let dir = tempfile::tempdir().expect("temp dir");

        log::commit(dir.path(), 0, &[make_add("data/v0.parquet")], CommitOptions::default()).unwrap();
        log::commit(dir.path(), 1, &[make_add("data/v1.parquet")], CommitOptions::default()).unwrap();

        let future = "9999-12-31T23:59:59Z";
        let resolved = version_at_timestamp(dir.path(), future).expect("resolve future timestamp");
        assert_eq!(resolved, 1);
    }
}
