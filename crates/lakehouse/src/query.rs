use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use arrow::util::pretty::print_batches;
use datafusion::datasource::listing::{ListingTable, ListingTableConfig, ListingTableUrl};
use datafusion::prelude::*;

use crate::snapshot;

/// Execute a SQL query against the table snapshot at `version`.
///
/// If `version` is None, the latest committed version is used.
/// If `explain` is true, the physical plan is printed instead of row data.
pub async fn sql(
    table_dir: &Path,
    version: Option<u64>,
    query: &str,
    explain: bool,
) -> anyhow::Result<()> {
    // 1. Resolve version
    let ver = match version {
        Some(v) => v,
        None => snapshot::latest_version(table_dir)?
            .context("table has no committed versions yet")?,
    };

    // 2. Reconstruct snapshot → list of active Parquet file paths
    let snap = snapshot::read(table_dir, ver)?;
    anyhow::ensure!(!snap.files.is_empty(), "snapshot at version {ver} contains no files");

    // 3. Build ListingTableUrls from absolute paths
    let table_urls: Vec<ListingTableUrl> = snap
        .files
        .iter()
        .map(|f| {
            let abs = table_dir.join(&f.path);
            ListingTableUrl::parse(abs.to_string_lossy().as_ref())
                .map_err(|e| anyhow::anyhow!("invalid table URL: {e}"))
        })
        .collect::<anyhow::Result<_>>()?;

    // 4. Build session + infer schema + options from the file set
    let ctx = SessionContext::new();
    let state = ctx.state();

    let config = ListingTableConfig::new_with_multi_paths(table_urls)
        .infer(&state)
        .await
        .context("failed to infer schema from snapshot files")?;

    let table = Arc::new(ListingTable::try_new(config).context("failed to create listing table")?);

    // Register as "t" so user SQL can reference it by name
    ctx.register_table("t", table)
        .context("failed to register table")?;

    // 5. Execute query (or EXPLAIN variant)
    let effective_query = if explain {
        format!("EXPLAIN {query}")
    } else {
        query.to_string()
    };

    let df = ctx
        .sql(&effective_query)
        .await
        .context("SQL parse/plan error")?;

    let batches = df.collect().await.context("query execution failed")?;

    // 6. Print results
    print_batches(&batches).context("failed to format results")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{input, log, snapshot, writer};
    use arrow::array::Int64Array;
    use datafusion::datasource::listing::{ListingTable, ListingTableConfig, ListingTableUrl};
    use std::collections::HashMap;
    use std::sync::Arc;

    async fn setup_table(table_dir: &Path, csvs: &[&str]) {
        let csv_dir = tempfile::tempdir().expect("csv dir");
        for (i, csv_data) in csvs.iter().enumerate() {
            let csv_path = csv_dir.path().join(format!("input{i}.csv"));
            std::fs::write(&csv_path, csv_data).expect("write csv");
            let batch = input::csv_to_batch(&csv_path).expect("csv_to_batch");
            let row_count = batch.num_rows() as u64;
            let abs_path = writer::write_batch(table_dir, &batch).expect("write_batch");
            let size = std::fs::metadata(&abs_path).unwrap().len();
            let rel_path = abs_path
                .strip_prefix(table_dir)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let version = snapshot::next_version(table_dir).expect("next_version");
            log::commit(
                table_dir,
                version,
                &[log::Action::Add(log::AddFile {
                    path: rel_path,
                    size,
                    row_count,
                    partition_values: HashMap::new(),
                })],
                log::CommitOptions::default(),
            )
            .expect("commit");
        }
    }

    /// Write 3 rows, query COUNT(*), assert result = 3.
    #[tokio::test]
    async fn sql_count_matches_written_rows() {
        let dir = tempfile::tempdir().expect("temp dir");
        let table_dir = dir.path();

        setup_table(table_dir, &["id,value\n1,a\n2,b\n3,c\n"]).await;

        // Verify via public sql() — no panic = query ran
        sql(table_dir, None, "SELECT COUNT(*) AS n FROM t", false)
            .await
            .expect("sql query failed");

        // Verify the actual count value by running through DataFusion directly
        let snap = snapshot::read(table_dir, 0).unwrap();
        let ctx = SessionContext::new();
        let state = ctx.state();
        let urls: Vec<ListingTableUrl> = snap
            .files
            .iter()
            .map(|f| ListingTableUrl::parse(table_dir.join(&f.path).to_string_lossy().as_ref()).unwrap())
            .collect();
        let cfg = ListingTableConfig::new_with_multi_paths(urls).infer(&state).await.unwrap();
        ctx.register_table("t", Arc::new(ListingTable::try_new(cfg).unwrap())).unwrap();
        let batches = ctx.sql("SELECT COUNT(*) AS n FROM t").await.unwrap().collect().await.unwrap();
        let count = Int64Array::from(batches[0].column(0).to_data()).value(0);
        assert_eq!(count, 3);
    }

    /// Two commits (3 + 2 rows) — COUNT(*) across both files must equal 5.
    #[tokio::test]
    async fn sql_count_across_multiple_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let table_dir = dir.path();

        setup_table(table_dir, &["id,value\n1,a\n2,b\n3,c\n", "id,value\n4,d\n5,e\n"]).await;

        sql(table_dir, None, "SELECT COUNT(*) AS n FROM t", false)
            .await
            .expect("multi-file sql failed");

        let snap = snapshot::read(table_dir, 1).unwrap();
        let ctx = SessionContext::new();
        let state = ctx.state();
        let urls: Vec<ListingTableUrl> = snap
            .files
            .iter()
            .map(|f| ListingTableUrl::parse(table_dir.join(&f.path).to_string_lossy().as_ref()).unwrap())
            .collect();
        let cfg = ListingTableConfig::new_with_multi_paths(urls).infer(&state).await.unwrap();
        ctx.register_table("t", Arc::new(ListingTable::try_new(cfg).unwrap())).unwrap();
        let batches = ctx.sql("SELECT COUNT(*) AS n FROM t").await.unwrap().collect().await.unwrap();
        let count = Int64Array::from(batches[0].column(0).to_data()).value(0);
        assert_eq!(count, 5);
    }

    /// --explain flag: query should return a plan, not data rows.
    #[tokio::test]
    async fn sql_explain_does_not_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        let table_dir = dir.path();

        setup_table(table_dir, &["id,value\n1,a\n"]).await;

        sql(table_dir, None, "SELECT * FROM t", true)
            .await
            .expect("explain query failed");
    }
}
