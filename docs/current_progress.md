# Lakehouse — Current Progress

_Last updated: 2026-04-09_ (W1-4 complete)

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

---

## Current State of Files

| File | Status | Notes |
|------|--------|-------|
| `src/main.rs` | Active | `doctor` and `write --table --input --version` wired |
| `src/layout.rs` | Complete | Directory names + log version formatting |
| `src/writer.rs` | Complete | Append-only Parquet writer with ULID naming + round-trip test |
| `src/input.rs` | Complete | CSV → Arrow `RecordBatch` loader with schema inference |
| `src/log.rs` | Complete | `AddFile`, `Action` enum, NDJSON `commit()` + 2 tests |
| `crates/lakehouse/Cargo.toml` | Active | `arrow`, `parquet`, `ulid`, `serde`, `serde_json` added |

---

## Weeks Still Ahead (high-level)

| Week | Issues | Topics |
|------|--------|--------|
| W1 | W1-5, W1-6 | Snapshot replay, first DataFusion query | ← next
| W2 | W2-1 → W2-4 | RemoveFile, atomic commit, CommitInfo, time travel |
| W3 | W3-1 → W3-3 | Metadata action, schema evolution, checkpointing |
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
