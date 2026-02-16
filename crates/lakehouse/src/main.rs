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
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Doctor => {
            println!("Ok");
        }
    }
    Ok(())
}
