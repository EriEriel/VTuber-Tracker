mod models;
mod routes;

use clap::{Parser, Subcommand};
use crate::routes::{fetch_vtubers, jump_to, create_vtuber_channel};

#[derive(Parser)]
#[command(
    name = "oshihub", 
    about = "Track and jump to your favorite VTubers"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Open a VTuber's channel in your browser
    Jump {
        /// Name (or partial name) of the VTuber to jump to
        name: String,
    },
    /// List all VTubers
    #[command(alias = "l")]
    List,
    /// Create a new VTuber channel from a URL
    #[command(alias = "c")]
    Create {
        /// URL of the VTuber channel to create
        url: String,
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        // TODO: Jump to is not implement or test properly yet.
        Commands::Jump { name } => jump_to(&name).await?,
        Commands::List => {
            let vtubers = fetch_vtubers().await?;
            vtubers.iter().for_each(|v| {
                println!(
                    "{} ({}) - {}",
                    v.english_name, v.name, v.platform_channel_id
                );
            });
        }
        Commands::Create { url } => {
            create_vtuber_channel(&url).await?;
        }
    }

    Ok(())
}
