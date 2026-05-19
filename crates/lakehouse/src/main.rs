mod input;
mod layout;
mod log;
mod query;
mod snapshot;
mod writer;

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "lakehouse")]
#[command(about = "A production-style mini lakehouse in Rust using Apache Arrow and DataFusion.")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Doctor,
    Write {
        /// Path to the table directory (will be created if absent)
        #[arg(long)]
        table: PathBuf,
        /// Path to the CSV input file
        #[arg(long)]
        input: PathBuf,
        /// Optional idempotency token embedded in CommitInfo (W2-3)
        #[arg(long)]
        txn_id: Option<String>,
        /// Optional application identifier embedded in CommitInfo (W2-3)
        #[arg(long)]
        app_id: Option<String>,
    },
    Snapshot {
        /// Path to the table directory
        #[arg(long)]
        table: PathBuf,
        /// Exact version to read; mutually exclusive with --timestamp
        #[arg(long)]
        version: Option<u64>,
        /// RFC 3339 timestamp — resolves to the latest version committed at or before this time (W2-4)
        #[arg(long)]
        timestamp: Option<String>,
    },
    Sql {
        /// Path to the table directory
        #[arg(long)]
        table: PathBuf,
        /// Exact snapshot version to query; mutually exclusive with --timestamp
        #[arg(long)]
        version: Option<u64>,
        /// RFC 3339 timestamp for time-travel queries (W2-4)
        #[arg(long)]
        timestamp: Option<String>,
        /// SQL query string (table is always named "t")
        #[arg(long)]
        query: String,
        /// Print the physical query plan instead of row data
        #[arg(long, default_value_t = false)]
        explain: bool,
    },
}

/// Resolve a snapshot version from the (version, timestamp) flag pair.
///
/// Rules:
/// - Both set         → error (ambiguous)
/// - Only version     → use it directly
/// - Only timestamp   → resolve via CommitInfo scan
/// - Neither          → latest committed version
fn resolve_version(
    table: &std::path::Path,
    version: Option<u64>,
    timestamp: Option<&str>,
) -> anyhow::Result<u64> {
    match (version, timestamp) {
        (Some(_), Some(_)) => {
            anyhow::bail!("--version and --timestamp are mutually exclusive; specify one or neither")
        }
        (Some(v), None) => Ok(v),
        (None, Some(ts)) => snapshot::version_at_timestamp(table, ts),
        (None, None) => snapshot::latest_version(table)?.context("table has no committed versions yet"),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Doctor => {
            println!("Ok");
        }
        Commands::Write { table, input, txn_id, app_id } => {
            let batch = input::csv_to_batch(&input)?;
            let row_count = batch.num_rows() as u64;

            let abs_path = writer::write_batch(&table, &batch)?;

            let size = std::fs::metadata(&abs_path)
                .with_context(|| format!("stat failed: {}", abs_path.display()))?
                .len();

            let rel_path = abs_path
                .strip_prefix(&table)
                .context("file path is not under table dir")?
                .to_string_lossy()
                .into_owned();

            let version = snapshot::next_version(&table)?;

            let action = log::Action::Add(log::AddFile {
                path: rel_path,
                size,
                row_count,
                partition_values: HashMap::new(),
            });

            log::commit(
                &table,
                version,
                &[action],
                log::CommitOptions {
                    operation: "write".to_string(),
                    txn_id,
                    app_id,
                },
            )?;

            println!(
                "wrote {} rows → {} (committed as version {})",
                row_count,
                abs_path.display(),
                version
            );
        }
        Commands::Snapshot { table, version, timestamp } => {
            let ver = resolve_version(&table, version, timestamp.as_deref())?;
            let snap = snapshot::read(&table, ver)?;

            println!("snapshot at version {}", snap.version);
            println!("{} file(s):", snap.files.len());
            for f in &snap.files {
                println!("  {} ({} rows, {} bytes)", f.path, f.row_count, f.size);
            }
        }
        Commands::Sql { table, version, timestamp, query, explain } => {
            let ver = resolve_version(&table, version, timestamp.as_deref())?;
            query::sql(&table, Some(ver), &query, explain).await?;
        }
    }
    Ok(())
}
