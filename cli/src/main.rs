mod models;
mod routes;

use clap::{Parser, Subcommand};
use crate::routes::{
    fetch_vtubers,
    fetch_vtuber_detail,
    jump_to,
    create_vtuber_channel,
    lookup_by_name,
    delete_vtuber_channel,
    sync_vtuber_channels,
    print_thumbnail,
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
        /// Max number of streams/clips to show per VTuber (capped at 10)
        #[arg(long, default_value_t = 10)]
        limit: usize,
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
        Commands::Lookup { name, limit } => {
            let vtubers = lookup_by_name(&name).await?;
            if vtubers.is_empty() {
                println!("No VTuber matching '{}' found", name);
            }

            let limit = limit.min(10);

            for v in &vtubers {
                println!(
                    "{} ({}) - {}",
                    v.english_name, v.name, v.platform_channel_id
                );

                if let Err(err) = print_thumbnail(&v.photo).await {
                    eprintln!("  (could not render thumbnail: {err})");
                }

                let detail = fetch_vtuber_detail(&v.id).await?;
                let is_live = detail.streams.iter().any(|s| s.status == "live");
                println!("  Status: {}", if is_live { "LIVE" } else { "offline" });

                println!("  Recent streams:");
                if detail.streams.is_empty() {
                    println!("    (none)");
                } else {
                    for s in detail.streams.iter().take(limit) {
                        println!("    [{}] {} - {}", s.status, s.title, s.url);
                    }
                }

                println!("  Recent clips:");
                if detail.clips.is_empty() {
                    println!("    (none)");
                } else {
                    for c in detail.clips.iter().take(limit) {
                        println!("    {} ({} views) - {}", c.title, c.view_count, c.url);
                    }
                }

                println!();
            }
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
