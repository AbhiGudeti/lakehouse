mod input;
mod layout;
mod log;
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
        /// Log version to commit this write as
        #[arg(long)]
        version: u64,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Doctor => {
            println!("Ok");
        }
        Commands::Write { table, input, version } => {
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

            // 4. Commit an AddFile action to the log
            let action = log::Action::Add(log::AddFile {
                path: rel_path,
                size,
                row_count,
                partition_values: HashMap::new(),
            });
            log::commit(&table, version, &[action])?;

            println!(
                "wrote {} rows → {} (log version {})",
                row_count,
                abs_path.display(),
                version
            );
        }
    }
    Ok(())
}
