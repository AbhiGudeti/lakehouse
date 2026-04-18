use std::fs;
use std::path::Path;

use anyhow::Context;

use crate::layout::{format_log_version, LOG_DIR};
use crate::log::{Action, AddFile};

/// The reconstructed state of a table at a specific version.
///
/// For now this is just the list of active data files.
/// It will grow to include schema (W3-1) and partition info.
#[derive(Debug)]
pub struct Snapshot {
    pub version: u64,
    pub files: Vec<AddFile>,
}

/// Replay the transaction log from version 0 up to and including `version`,
/// returning the set of active files at that point in time.
///
/// Each log file is read line-by-line (NDJSON). Only `Action::Add` is handled
/// for now — `Remove` (W2-1) will subtract from `files`.
pub fn read(table_dir: &Path, version: u64) -> anyhow::Result<Snapshot> {
    let log_dir = table_dir.join(LOG_DIR);
    let mut files: Vec<AddFile> = Vec::new();

    for v in 0..=version {
        let filename = format!("{}.json", format_log_version(v));
        let log_path = log_dir.join(&filename);

        // A missing log file for version v means the table has no commit at
        // that version yet — stop here rather than skipping silently.
        let contents = fs::read_to_string(&log_path)
            .with_context(|| format!("log file missing for version {v}: {}", log_path.display()))?;

        for line in contents.lines() {
            let action: Action =
                serde_json::from_str(line).with_context(|| format!("invalid JSON in {filename}: {line}"))?;

            match action {
                Action::Add(add_file) => files.push(add_file),
            }
        }
    }

    Ok(Snapshot { version, files })
}

/// Scan `_log/` to find the highest committed version number.
///
/// Returns `None` if no log files exist (table is empty / uninitialized).
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

        // Only consider files matching the pattern <20 digits>.json
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::{self, Action};
    use std::collections::HashMap;

    fn make_add(path: &str) -> Action {
        Action::Add(AddFile {
            path: path.to_string(),
            size: 1024,
            row_count: 10,
            partition_values: HashMap::new(),
        })
    }

    #[test]
    fn snapshot_collects_all_add_files() {
        let dir = tempfile::tempdir().expect("temp dir");

        log::commit(dir.path(), 0, &[make_add("data/file0.parquet")]).unwrap();
        log::commit(dir.path(), 1, &[make_add("data/file1.parquet")]).unwrap();

        let snap = read(dir.path(), 1).expect("read snapshot");

        assert_eq!(snap.version, 1);
        assert_eq!(snap.files.len(), 2);
        assert_eq!(snap.files[0].path, "data/file0.parquet");
        assert_eq!(snap.files[1].path, "data/file1.parquet");
    }

    #[test]
    fn snapshot_at_version_0_has_only_first_file() {
        let dir = tempfile::tempdir().expect("temp dir");

        log::commit(dir.path(), 0, &[make_add("data/file0.parquet")]).unwrap();
        log::commit(dir.path(), 1, &[make_add("data/file1.parquet")]).unwrap();

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

        log::commit(dir.path(), 0, &[make_add("data/file0.parquet")]).unwrap();
        assert_eq!(next_version(dir.path()).unwrap(), 1);

        log::commit(dir.path(), 1, &[make_add("data/file1.parquet")]).unwrap();
        assert_eq!(next_version(dir.path()).unwrap(), 2);
    }
}
