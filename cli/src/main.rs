mod models;
mod routes;

use clap::{Parser, Subcommand};
use crate::routes::{
    fetch_vtubers,
    jump_to,
    create_vtuber_channel,
    lookup_by_name,
    delete_vtuber_channel,
    sync_vtuber_channels,
};

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
    #[command(alias = "j")]
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
    },
    /// Lookup VTubers by name
    #[command(alias = "lk")]
    Lookup {
        /// Name (or partial name) of the VTuber to look up
        name: String,
    },
    /// Delete a VTuber channel by name
    #[command(alias = "d")]
    Delete {
        /// Name (or partial name) of the VTuber to delete
        name: String,
    },
    /// Sync VTuber channels with the database
    #[command(alias = "s")]
    Sync {
        name: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
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
        //TODO: Show Stream, live status(Not properly implement yet), and clip if possible. deafault show 10 of each along
        //side with title and thumnail(With kitty graphic protocal), allow flag to show in <= 10 of each
        Commands::Lookup { name } => {
            let vtubers = lookup_by_name(&name).await?;
            vtubers.iter().for_each(|v| {
                println!(
                    "{} ({}) - {}",
                    v.english_name, v.name, v.platform_channel_id
                );
            });
        }
        Commands::Delete { name } => {
            delete_vtuber_channel(&name).await?;
        },
        Commands::Sync { name } => {
            sync_vtuber_channels(Some(&name)).await?;
        }
    }

    Ok(())
}
