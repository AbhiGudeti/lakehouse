mod input;
mod layout;
mod log;
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
    },
    Snapshot {
        /// Path to the table directory
        #[arg(long)]
        table: PathBuf,
        /// Version to read; defaults to latest committed version
        #[arg(long)]
        version: Option<u64>,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Doctor => {
            println!("Ok");
        }
        Commands::Write { table, input } => {
            // 1. Load CSV → RecordBatch
            let batch = input::csv_to_batch(&input)?;
            let row_count = batch.num_rows() as u64;

            // 2. Write Parquet file; returns absolute path
            let abs_path = writer::write_batch(&table, &batch)?;

            // 3. Compute file size and relative path for the log entry
            let size = std::fs::metadata(&abs_path)
                .with_context(|| format!("stat failed: {}", abs_path.display()))?
                .len();

            let rel_path = abs_path
                .strip_prefix(&table)
                .context("file path is not under table dir")?
                .to_string_lossy()
                .into_owned();

            // 4. Determine version automatically — no more manual --version flag
            let version = snapshot::next_version(&table)?;

            // 5. Commit an AddFile action to the log
            let action = log::Action::Add(log::AddFile {
                path: rel_path,
                size,
                row_count,
                partition_values: HashMap::new(),
            });
            log::commit(&table, version, &[action])?;

            println!(
                "wrote {} rows → {} (committed as version {})",
                row_count,
                abs_path.display(),
                version
            );
        }
        Commands::Snapshot { table, version } => {
            let ver = match version {
                Some(v) => v,
                None => snapshot::latest_version(&table)?
                    .context("table has no committed versions yet")?,
            };

            let snap = snapshot::read(&table, ver)?;

            println!("snapshot at version {}", snap.version);
            println!("{} file(s):", snap.files.len());
            for f in &snap.files {
                println!("  {} ({} rows, {} bytes)", f.path, f.row_count, f.size);
            }
        }
    }
    Ok(())
}
