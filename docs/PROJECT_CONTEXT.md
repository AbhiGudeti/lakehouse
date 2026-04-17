# Mini Lakehouse — Detailed Project Context

---

## Week 1 (Feb 16–22): Repo scaffolding + first vertical slice

### W1-1: Initialize repo, tooling, and CI
**Acceptance:**
- `cargo fmt`, `cargo clippy`, and `cargo test` run in CI on push/PR
- `README.md` has "How to run tests" and "How to build"
- `docker/` folder exists (even if empty this week)

### W1-2: Define table directory layout + conventions
**Acceptance:**
- `docs/architecture.md` includes directory structure: `data/`, `_log/`, optional `_checkpoints/`
- Define log version naming: 20-digit zero-padded files
- Define partitioning convention (even if "no partitions" for now)

### W1-3: Implement Parquet writer (append-only)
**Acceptance:**
- CLI command: `lakehouse write --table <path> --input <csv/json>`
- Produces Parquet file(s) with deterministic naming
- Unit test: write then read back row count matches input

### W1-4: Implement Commit Log v0 (AddFile only)
**Acceptance:**
- `commit(version)` creates the next log file
- Log entry includes: file path, row count, file size, partition values (empty ok)
- Unit test: after commit, log file exists and is parseable JSON

### W1-5: Implement snapshot reconstruction (replay log)
**Acceptance:**
- API/CLI: `lakehouse snapshot --version N`
- Returns list of active Parquet files
- Test: version N has superset of version N-1 after append

### W1-6: First query path (DataFusion reads snapshot files)
**Acceptance:**
- CLI: `lakehouse sql --table <path> --version N --query "SELECT COUNT(*) ..."`
- Works end-to-end on local sample data
- `EXPLAIN` option prints plan

---

## Week 2 (Feb 23–Mar 1): Correct commit protocol + time travel

### W2-1: Add RemoveFile action + rewrite semantics
**Acceptance:**
- Snapshot reconstruction respects `RemoveFile`
- Test: write A, commit; remove A, commit; snapshot shows no A

### W2-2: Atomic-ish commit on local FS
**Acceptance:**
- Commit writes to temp then renames to final log filename
- If process crashes mid-commit, table remains readable at previous version
- Add test that simulates partial file write (best-effort)

### W2-3: Add CommitInfo metadata
**Acceptance:**
- Each log file has a `CommitInfo` entry
- CLI supports `--txn-id` and `--app-id`
- Snapshot reader can ignore unknown fields safely

### W2-4: Implement time travel by version + timestamp
**Acceptance:**
- CLI: `--version N` works
- CLI: `--timestamp "2026-03-01T12:00:00Z"` picks correct version
- Test: timestamp between commits resolves to expected snapshot

---

## Week 3 (Mar 2–Mar 8): Schema evolution + checkpointing v1

### W3-1: Add Metadata action (schema + config)
**Acceptance:**
- Metadata includes schema JSON + partition columns + table id
- Reader returns schema for snapshot version
- Test: schema retrieved equals writer's schema

### W3-2: Schema evolution v1 (add column)
**Acceptance:**
- Write v1 schema; commit
- Update schema to add nullable column; commit
- Query reads old files with nulls for new column
- Document compatibility rules in `docs/schema.md`

### W3-3: Checkpointing v1 (fast snapshot load)
**Acceptance:**
- After K commits, `lakehouse checkpoint` generates checkpoint file
- Snapshot load uses checkpoint + remaining logs
- Benchmark: snapshot load time decreases vs replay-only

---

## Week 4 (Mar 9–Mar 15): Streaming ingestion v1

### W4-1: Docker compose for Redpanda + console
**Acceptance:**
- `docker-compose up` starts Redpanda
- `scripts/produce_events.py` can publish events to a topic
- README includes instructions

### W4-2: Streaming ingestor (micro-batch)
**Acceptance:**
- CLI: `lakehouse ingest --topic X --table <path> --batch-seconds 5`
- Produces new commits continuously
- Stores consumed offset ranges in commit metadata

### W4-3: Basic "query while ingesting" demo
**Acceptance:**
- Demo script: run ingest + run query loop every 10s
- Query never crashes / never reads partial commit
- Save short demo GIF or terminal recording (optional)

---

## Week 5 (Mar 16–Mar 22): Restart safety + fault injection

### W5-1: Idempotent batch IDs + deterministic file naming
**Acceptance:**
- Each micro-batch has unique batch id
- File naming includes batch id; reprocessing doesn't create new files
- Test: re-run same batch commit; snapshot unchanged

### W5-2: Exactly-once-ish invariant enforcement
**Acceptance:**
- Offsets only recorded after commit succeeds
- On restart, consumer resumes from last committed offsets
- Test: kill process mid-batch; restart; no duplicate rows

### W5-3: Fault injection tests
**Acceptance:**
- Crash before file write → no commit, no data
- Crash after file write before commit → no data visible
- Crash after commit → data visible once
- Document in `docs/correctness.md`

---

## Week 6 (Mar 23–Mar 29): Baseline benchmark harness

### W6-1: TPC-H dataset generation + loader
**Acceptance:**
- `scripts/gen_tpch.sh` produces data
- `lakehouse load-tpch --scale 1`
- Tables created with commit logs

### W6-2: Benchmark runner v1
**Acceptance:**
- `bench/run_bench.py` (or Rust binary) executes query list
- Records: latency, rows returned, bytes scanned (if available), plan
- Outputs `benchmarks/baseline.json`

### W6-3: EXPLAIN plan capture + diff tooling
**Acceptance:**
- Each bench stores EXPLAIN plan text
- `bench/plan_diff.py` shows before/after diffs per query

---

## Week 7 (Mar 30–Apr 5): Predicate pushdown

### W7-1: Parquet predicate pushdown plumbing
**Acceptance:**
- Explain plan indicates pushdown
- For selective queries, files/row groups read decreases (or scan time drops)
- Bench results saved to `benchmarks/pushdown.json`

### W7-2: Add "bytes scanned" or "row groups read" metric
**Acceptance:**
- Metric recorded per query (best available proxy)
- Included in benchmark JSON

---

## Week 8 (Apr 6–Apr 12): Column pruning + stats extraction

### W8-1: Column pruning into Parquet scan
**Acceptance:**
- Explain plan shows projection pushdown
- Wide-table query improves measurably
- Bench saved `benchmarks/col_prune.json`

### W8-2: File statistics extraction
**Acceptance:**
- `AddFile` action includes stats blob
- Stats generated during write
- Unit test validates stats on known dataset

---

## Week 9 (Apr 13–Apr 19): Stats-based file pruning

### W9-1: File-level data skipping using stats
**Acceptance:**
- For predicate on a column with stats, engine skips files that can't match
- Record metric: files considered vs files scanned
- Bench saved `benchmarks/file_skip.json`

### W9-2: Stats correctness tests
**Acceptance:**
- Test where predicate should skip all files → returns 0 rows quickly
- Test where predicate matches subset → only those files scanned

---

## Week 10 (Apr 20–Apr 26): Join ordering + cost model

### W10-1: Cardinality estimator v1
**Acceptance:**
- Produces estimated rows for scan and selectivity
- Logged in explain/trace output

### W10-2: Join reorder optimizer rule (greedy)
**Acceptance:**
- Reorders joins for 3+ table joins
- Explain plan shows new join order
- At least 2 queries improve (or document when it fails)

### W10-3: Benchmark join improvements + ablation notes
**Acceptance:**
- `benchmarks/join_reorder.json`
- `docs/optimizer.md` includes "what improved and why"

---

## Week 11 (Apr 27–May 3): Compaction

### W11-1: Compaction planner (select candidates)
**Acceptance:**
- Identifies small files by size threshold
- Outputs compaction plan (list of files to rewrite)

### W11-2: Compaction executor (rewrite + transactional commit)
**Acceptance:**
- Rewrites N small files into M larger files
- Commits `RemoveFile`/`AddFile` atomically
- Snapshot before compaction still queryable (time travel)

### W11-3: Compaction benchmark + storage amplification metrics
**Acceptance:**
- Report: file count before/after, storage size delta, query latency delta
- Saved in `benchmarks/compaction.json`

---

## Week 12 (May 4–May 10): Data quality framework

### W12-1: Expectations spec + runner
**Acceptance:**
- YAML/JSON expectations per table
- Checks: null %, range, regex, uniqueness
- Runner outputs validation report

### W12-2: Persist validation results per snapshot
**Acceptance:**
- Validation results stored with snapshot version and timestamp
- API returns latest status + history

### W12-3: "Fail closed" option
**Acceptance:**
- Config option: block commit if expectations fail (or mark snapshot invalid)
- Demonstrate with an intentionally bad batch

---

## Week 13 (May 11–May 17): Drift + lineage

### W13-1: Drift detection (PSI or KS)
**Acceptance:**
- Compare baseline snapshot vs current snapshot
- Output drift scores per column
- Store drift results per snapshot

### W13-2: Lineage capture for ingestion + queries
**Acceptance:**
- Ingestion job writes lineage: (job_run → snapshot)
- Query endpoint logs lineage: (query_id → snapshot(s))
- Simple graph store in SQLite/Postgres (your choice)

### W13-3: Lineage query API
**Acceptance:**
- API endpoint: "what produced this snapshot?"
- API endpoint: "what queries touched snapshot version N?"

---

## Week 14 (May 18–May 24): UI + observability

### W14-1: Backend API consolidation
**Acceptance:**
- Single service exposing: tables, snapshots, files, schema, benchmark results, quality + drift + lineage

### W14-2: Web UI v1 (Table browser + commit history)
**Acceptance:**
- View tables
- View commit log timeline
- Click version → show files + schema

### W14-3: Web UI v2 (Quality + drift + lineage views)
**Acceptance:**
- Quality dashboard per table/version
- Drift chart/table
- Lineage graph view (even a simple node-edge list is fine)

### W14-4: Observability basics
**Acceptance:**
- Structured logs with request IDs
- Metrics: query latency, ingest throughput, compaction runtime
- (Optional) OTel traces if time

---

## Week 15 (May 25–May 31): Final polish, reports, and "recruiter proof"

### W15-1: Final benchmark suite + ablation table
**Acceptance:**
- One command produces full benchmark report
- Table shows improvements for: pushdown, pruning, file skipping, join reorder, compaction
- Report committed in `docs/benchmarks.md`

### W15-2: Reproducibility "fresh machine" runbook
**Acceptance:**
- README has: `docker-compose up`, `make load-data`, `make bench`, `make demo`
- Works from a clean clone

### W15-3: Engineering writeups
**Acceptance:**
- `docs/architecture.md` finalized (diagrams ok)
- `docs/correctness.md` includes failure tests + invariants
- `docs/optimizer.md` describes rules + plan diffs
- `docs/postmortems/` has at least 2 "what broke" writeups

### W15-4: Resume bullets + project "one slide"
**Acceptance:**
- `docs/resume_bullets.md` contains 4–6 bullets with numbers
- `docs/one_slide.md` includes: architecture diagram, benchmark highlights, key correctness guarantees

---

## Labels

**By week:** `week-01` through `week-15`

**By domain:** `storage`, `streaming`, `query-engine`, `optimizer`, `benchmark`, `governance`, `ui`, `docs`, `testing`

**By priority:** `must-have`, `stretch`

---

## Stretch Epics (only if ahead of schedule)

- **Object storage mode (S3):** commit protocol that works without atomic rename
- **Row-group bloom filters / zonemaps:** stronger skipping beyond min/max stats

---

## Golden Rules
- **Log = truth** — the transaction log is the authoritative record of what data exists
- **Data immutable** — files in `data/` are never modified in place
- **No silent failure** — every error surfaces explicitly
- **Measure everything** — benchmarks are first-class
- **Clean separation of layers** — storage, log, query, streaming are distinct boundaries
