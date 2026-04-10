use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use arrow::csv::ReaderBuilder;
use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;

/// Load a CSV file into a single `RecordBatch`.
///
/// Schema is inferred from the first 100 rows (Arrow's default inference).
/// All rows are collected into one batch — acceptable for W1-3 scale.
pub fn csv_to_batch(path: &Path) -> anyhow::Result<RecordBatch> {
    let file =
        fs::File::open(path).with_context(|| format!("failed to open CSV: {}", path.display()))?;

    // Infer schema by peeking at up to 100 rows
    let format = arrow::csv::reader::Format::default().with_header(true);
    let (schema, _) = format
        .infer_schema(&file, Some(100))
        .context("failed to infer CSV schema")?;

    // Reopen — infer_reader_schema consumes the file's cursor position
    let file = fs::File::open(path)
        .with_context(|| format!("failed to reopen CSV: {}", path.display()))?;

    let schema: SchemaRef = Arc::new(schema);

    let mut reader = ReaderBuilder::new(Arc::clone(&schema))
        .with_header(true)
        .build(file)
        .context("failed to build CSV reader")?;

    // Collect all batches (ReaderBuilder yields chunks; merge into one)
    let mut batches = Vec::new();
    for result in &mut reader {
        batches.push(result.context("error reading CSV batch")?);
    }

    anyhow::ensure!(!batches.is_empty(), "CSV file produced no record batches");

    // Concatenate all chunks into a single RecordBatch
    let batch = arrow::compute::concat_batches(&schema, &batches)
        .context("failed to concatenate CSV batches")?;

    Ok(batch)
}
