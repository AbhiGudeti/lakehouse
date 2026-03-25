# Architecture

## On-disk Table Layout

Each table is a directory containing immutable Parquet data files and an append-only transaction log.

<table_root>/
data/ # immutable Parquet data files (never modified in place)
_log/ # transaction log (append only, versioned JSON files)
_checkpoints/ # optional -- checkpoint files for faster snapshot reconstruction
_tmpl/ # optional -- staging area for atomicish commits

### data/
- stores Parquet files only 
- these data files are immutable; writers can create new files and commit them via the log 

### _log/
- the transaction log here is the **single source of truth** for table state
- a snapshot at version n is defined by replaying the log actions from versions 0..n 

#### Log version naming 
Log files will be named as **20-digit, zero-padded base-10 integers** with `.json` extension: 
- `00000000000000000000.json`
- `00000000000000000001.json`
- ...

The lexicographical ordering must match the numeric ordering. 

### _checkpoints/ (optional)
- Derived artifacts to accelerate snapshot loading.
- Readers load the newest checkpoint <= requested version, then replay subsequent log files.

## Partitioning convention

Tables may be unpartitioned or partitioned.

### Unpartitioned
`data/<file>.parquet`

### Partitioned (Hive-style directory encoding)
`data/<col1>=<value1>/<col2>=<value2>/<file>.parquet`

Partition columns are ordered as defined in table metadata.
Null partition values are encoded as `__NULL__`.

## Table state & integrity invariants
- A file is part of the table **iff** it is referenced by the active snapshot reconstructed from `_log/`.
- Files present in `data/` but not referenced by the log are ignored.
- If a file is referenced by a committed snapshot but missing/corrupt on disk, reads must fail loudly.
- Readers never use directory listing of `data/` to determine table contents (layout is physical only).