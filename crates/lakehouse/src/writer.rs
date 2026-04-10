use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;
use ulid::Ulid;

use crate::layout::DATA_DIR;

/// Write `batch` into `<table_dir>/data/<ulid>.parquet`.
///
/// Returns the path of the newly created file.
pub fn write_batch(table_dir: &Path, batch: &RecordBatch) -> anyhow::Result<PathBuf> {
    let data_dir = table_dir.join(DATA_DIR);
    fs::create_dir_all(&data_dir)
        .with_context(|| format!("failed to create data dir: {}", data_dir.display()))?;

    let filename = format!("{}.parquet", Ulid::new());
    let file_path = data_dir.join(&filename);

    let file = fs::File::create(&file_path)
        .with_context(|| format!("failed to create file: {}", file_path.display()))?;

    let props = WriterProperties::builder().build();
    let mut writer = ArrowWriter::try_new(file, Arc::clone(batch.schema_ref()), Some(props))
        .context("failed to create ArrowWriter")?;

    writer.write(batch).context("failed to write batch")?;
    writer.close().context("failed to close ArrowWriter")?;

    Ok(file_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Int64Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    #[test]
    fn round_trip_row_count() {
        // Build a minimal schema: id (Int64), name (Utf8)
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, false),
        ]));

        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int64Array::from(vec![1, 2, 3])),
                Arc::new(StringArray::from(vec!["alice", "bob", "carol"])),
            ],
        )
        .expect("valid RecordBatch");

        let dir = tempfile::tempdir().expect("temp dir");
        let path = write_batch(dir.path(), &batch).expect("write_batch");

        // Read back and assert row count
        let file = fs::File::open(&path).expect("open parquet");
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .expect("builder")
            .build()
            .expect("reader");

        let rows: usize = reader.map(|b| b.expect("batch").num_rows()).sum();

        assert_eq!(rows, 3);
    }
}
