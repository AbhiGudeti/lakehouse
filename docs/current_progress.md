# Lakehouse — Current Progress

_Last updated: 2026-05-19_ (W2-4 complete)

---

## What's Done

### W1-1: Initialize repo, tooling, and CI ✅
- Rust workspace with `crates/lakehouse` member
- GitHub Actions CI: `cargo fmt`, `cargo clippy`, `cargo test --all`
- `clap`-based CLI skeleton with a `doctor` subcommand
- `assert_cmd` integration test verifying `lakehouse doctor` prints `Ok`
- `anyhow` for error propagation, `rustfmt.toml` for formatting rules

### W1-2: Define table directory layout + conventions ✅
- `layout.rs` defines directory constants: `data/`, `_log/`, `_checkpoints/`, `_tmpl/`
- `format_log_version(u64) -> String` produces zero-padded 20-digit log filenames
- Unit tests covering edge cases (0, 1, max u64 representative)

### W1-3: Implement Parquet writer (append-only) ✅

- Added `arrow = "54"`, `parquet = "54"` (with `arrow` feature), `ulid = "1"` to `Cargo.toml`
- `writer.rs`: `write_batch(table_dir, batch) -> PathBuf` — creates `data/` if absent, generates a ULID filename, opens a `File`, constructs `ArrowWriter`, writes the batch as one Parquet row group, calls `close()` to flush the footer
- `input.rs`: `csv_to_batch(path) -> RecordBatch` — infers schema via `Format::infer_schema` (100-row peek), reopens file, streams CSV in 1024-row chunks via `ReaderBuilder`, concatenates all chunks into one `RecordBatch` with `concat_batches`
- `main.rs`: wired `write --table <path> --input <csv>` subcommand; delegates to `input::csv_to_batch` → `writer::write_batch`; prints row count and output path
- Round-trip unit test: manually constructs a 3-row `RecordBatch`, writes it, reads it back with `ParquetRecordBatchReaderBuilder`, asserts row count = 3
- All 3 tests pass: `layout::tests::formats_log_version_as_20_digits`, `writer::tests::round_trip_row_count`, `doctor_prints_ok`

### W1-4: Implement Commit Log v0 (AddFile only) ✅
- Added `serde = "1"` (with `derive` feature) and `serde_json = "1"` to `Cargo.toml`
- `log.rs`: `AddFile` struct — records `path` (relative to table root), `size` (bytes), `row_count`, `partition_values` (empty `HashMap` for now); serializes to `{"add":{...}}` via `#[serde(rename_all = "camelCase")]`
- `log.rs`: `Action` enum — single `Add(AddFile)` variant; designed to grow with `Remove`, `CommitInfo`, `Metadata` in future weeks
- `log.rs`: `commit(table_dir, version, actions)` — creates `_log/` if absent, writes each action as one NDJSON line to `_log/<version>.json`
- `main.rs`: `write` command extended with `--version <u64>`; after writing Parquet, calls `fs::metadata` for size, `strip_prefix` for relative path, then `log::commit`
- Two unit tests: (1) log file exists at correct path and every line parses as JSON with an `"add"` key; (2) metadata fields (`path`, `size`, `rowCount`) match what was committed
- All 5 tests pass

### W1-6: First query path (DataFusion reads snapshot files) ✅

- Added `datafusion = "46"` and `tokio = "1"` (rt-multi-thread + macros) to `Cargo.toml`
- `query.rs`: `sql(table_dir, version, query, explain) -> anyhow::Result<()>` (async)
  - Reconstructs snapshot via `snapshot::read`
  - Builds a `ListingTable` from `ListingTableConfig::new_with_multi_paths` — handles 1 or N Parquet files from the snapshot, schema inferred automatically
  - Registers the table as `"t"` in a fresh `SessionContext`
  - Prepends `EXPLAIN` to the query string when `--explain` is set
  - Collects `RecordBatch` results and prints via `arrow::util::pretty::print_batches`
- `main.rs`: converted to `async fn main` with `#[tokio::main]`; added `Sql { --table, --version, --query, --explain }` subcommand
- 3 new unit tests: single-file COUNT(*) correctness, multi-file COUNT(*) correctness, EXPLAIN path runs without error
- All 12 tests pass (11 unit + 1 integration)

### W1-5: Snapshot reconstruction (replay log) ✅
- `snapshot.rs`: `Snapshot` struct — holds `version: u64` and `files: Vec<AddFile>`; derived by replaying the log, never stored on disk
- `snapshot.rs`: `read(table_dir, version)` — iterates versions `0..=version`, opens each `_log/<v>.json`, deserializes every NDJSON line as an `Action`, accumulates `AddFile` entries into `files`; errors explicitly on missing log files (no silent skip)
- `snapshot.rs`: `latest_version(table_dir)` — reads `_log/` directory, parses filenames matching `<20 digits>.json`, returns the maximum version as `Option<u64>` (`None` if no commits exist yet)
- `snapshot.rs`: `next_version(table_dir)` — wraps `latest_version`; returns `0` for an empty table, `N+1` otherwise
- `main.rs`: `write` command drops `--version` flag; version now auto-derived from `next_version()` before every commit
- `main.rs`: new `snapshot --table <path> [--version N]` subcommand — resolves to latest version if `--version` omitted, prints each active file's path, row count, and size
- 4 unit tests: snapshot collects all AddFiles across versions; snapshot at version 0 excludes later files; `next_version` returns 0 for empty table; `next_version` increments after each commit
- All 9 tests pass

### W2-1: RemoveFile action + rewrite semantics ✅
- Added `RemoveFile` struct to `log.rs` — mirrors `AddFile` shape (`path`, `partition_values`); serializes as `{"remove":{...}}`
- Added `Action::Remove(RemoveFile)` variant to the `Action` enum
- `snapshot::read()` now handles `Action::Remove`: calls `files.retain(|f| f.path != remove_file.path)` to evict the file from the active set at that version
- `Action::CommitInfo` arm added as a no-op so the reader is not surprised by the new first-line metadata
- Tests: `remove_file_evicts_from_snapshot` (write A, remove A, snapshot is empty); `snapshot_before_remove_still_has_file` (snapshot at pre-remove version still sees A)

### W2-2: Atomic-ish commit on local FS ✅
- `commit()` now writes to `<version>.json.tmp` first, flushes + closes, then calls `fs::rename` to promote to `<version>.json`
- On POSIX, `rename(2)` is an atomic syscall: readers see either the complete file or nothing — never a partial write
- `latest_version()` only counts `.json` files with 20-digit stems, so orphaned `.tmp` files from crashed commits are invisible
- Test: `stale_tmp_file_does_not_affect_previous_version` — writes a garbage `.tmp` for version 1, asserts version 0 is still fully readable

### W2-3: CommitInfo metadata ✅
- Added `CommitInfo` struct: `timestamp` (RFC 3339 UTC), `operation`, `txn_id: Option<String>`, `app_id: Option<String>`
- `commit()` prepends a `CommitInfo` action as the first NDJSON line of every log file so readers can extract the timestamp without scanning data actions
- Added `CommitOptions` struct (with `Default`) forwarded from all call sites
- CLI `write` subcommand: `--txn-id` and `--app-id` optional flags wired through to `CommitOptions`
- Test: `commit_info_is_first_line_and_has_timestamp` — asserts first line is `commitInfo`, has a non-null timestamp, operation, txnId, appId

### W2-4: Time travel by version + timestamp ✅
- `snapshot::read_commit_timestamp(table_dir, version)` — scans a log file for the first `CommitInfo` action and returns its timestamp string
- `snapshot::version_at_timestamp(table_dir, timestamp)` — parses requested RFC 3339 timestamp, iterates all committed versions, returns the latest version whose `CommitInfo.timestamp ≤ requested`
- CLI `snapshot` and `sql` subcommands: `--timestamp "2026-03-01T12:00:00Z"` flag added; `resolve_version()` helper enforces mutual exclusivity of `--version` and `--timestamp`
- Tests: `version_at_timestamp_resolves_correctly` (timestamp between commits → v0); `version_at_timestamp_after_all_commits_gives_latest` (far-future timestamp → latest version)

---

## Current State of Files

| File | Status | Notes |
|------|--------|-------|
| `src/main.rs` | Active | `doctor`, `write --table --input`, `snapshot --table [--version]` wired |
| `src/layout.rs` | Complete | Directory names + log version formatting |
| `src/writer.rs` | Complete | Append-only Parquet writer with ULID naming + round-trip test |
| `src/input.rs` | Complete | CSV → Arrow `RecordBatch` loader with schema inference |
| `src/log.rs` | Complete | `AddFile`, `RemoveFile`, `CommitInfo`, `CommitOptions`, atomic `commit()` + 4 tests |
| `src/snapshot.rs` | Complete | `read()` handles Remove+CommitInfo; `version_at_timestamp()`; `latest/next_version()` + 8 tests |
| `src/query.rs` | Complete | `sql()` async fn via DataFusion `ListingTable` + 3 tests |
| `src/main.rs` | Complete | `write --txn-id --app-id`; `snapshot/sql --version/--timestamp`; `resolve_version()` |
| `crates/lakehouse/Cargo.toml` | Complete | `chrono` added for RFC 3339 timestamp handling |

---

## Weeks Still Ahead (high-level)

| Week | Issues | Topics |
|------|--------|--------|
| W1 | — | Complete ✅ |
| W2 | W2-1 → W2-4 | RemoveFile, atomic commit, CommitInfo, time travel | ✅
| W3 | W3-1 → W3-3 | Metadata action, schema evolution, checkpointing | ← next
| W4 | W4-1 → W4-3 | Redpanda Docker, streaming ingestor, query-while-ingesting demo |
| W5 | W5-1 → W5-3 | Idempotent batches, exactly-once offsets, fault injection |
| W6 | W6-1 → W6-3 | TPC-H loader, benchmark runner, EXPLAIN diff tooling |
| W7 | W7-1 → W7-2 | Predicate pushdown, bytes-scanned metric |
| W8 | W8-1 → W8-2 | Column pruning, file statistics extraction |
| W9 | W9-1 → W9-2 | File-level data skipping, stats correctness tests |
| W10 | W10-1 → W10-3 | Cardinality estimator, join reorder, optimizer benchmarks |
| W11 | W11-1 → W11-3 | Compaction planner, executor, benchmark |
| W12 | W12-1 → W12-3 | Data quality expectations, validation storage, fail-closed |
| W13 | W13-1 → W13-3 | Drift detection, lineage capture, lineage query API |
| W14 | W14-1 → W14-4 | Backend API, web UI v1+v2, observability |
| W15 | W15-1 → W15-4 | Final benchmarks, runbook, writeups, resume bullets |

---

## Golden Rules
- **Log = truth** — the transaction log is the authoritative record of what data exists
- **Data immutable** — files in `data/` are never modified in place
- **No silent failure** — every error surfaces explicitly
- **Measure everything** — benchmarks are first-class
- **Clean separation of layers** — storage, log, query, streaming are distinct boundaries
