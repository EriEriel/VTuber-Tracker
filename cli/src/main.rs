mod config;
mod models;
mod notify;
mod routes;
mod theme;
mod tui;
mod watch;

use clap::{Parser, Subcommand};
use crossterm::execute;
use crossterm::cursor::{MoveRight, MoveDown, MoveToColumn};
use std::io::stdout;
use crate::routes::{
    fetch_vtubers,
    fetch_vtuber_detail,
    fetch_live_vtubers,
    jump_to,
    create_vtuber_channel,
    lookup_by_name,
    delete_vtuber_channel,
    sync_vtuber_channels,
    print_thumbnail,
    print_stream_thumbnail,
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
    /// List VTubers who are currently live
    #[command(alias = "lv")]
    Live,
    /// Watch for VTubers going live and send desktop notifications
    #[command(alias = "w")]
    Watch {
        /// Seconds between polls (floor 15; overrides the config file)
        #[arg(long)]
        interval: Option<u64>,
        /// Notify for VTubers already live at startup instead of treating
        /// them as the quiet baseline
        #[arg(long)]
        notify_existing: bool,
    },
    /// Browse tracked VTubers in a full-screen terminal UI
    #[command(alias = "t")]
    Tui,
    /// Show the resolved backend URL and where it came from
    Config,
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
                    theme::name(&v.english_name),
                    theme::muted(&v.name),
                    theme::muted(&v.platform_channel_id)
                );
            });
        }
        Commands::Create { url } => match create_vtuber_channel(&url).await {
            Ok(()) => println!("Successfully created VTuber channel from {url}"),
            Err(err) => println!("Failed to create VTuber channel from {url}: {err}"),
        },
        Commands::Lookup { name, limit } => {
            let vtubers = lookup_by_name(&name).await?;
            if vtubers.is_empty() {
                println!("No VTuber matching '{}' found", name);
            }

            let limit = limit.min(10);

            for v in &vtubers {
                println!(
                    "{} ({}) - {}",
                    theme::name(&v.english_name),
                    theme::muted(&v.name),
                    theme::muted(&v.platform_channel_id)
                );

                if let Err(err) = print_thumbnail(&v.photo).await {
                    eprintln!("  (could not render thumbnail: {err})");
                }

                let detail = fetch_vtuber_detail(&v.id).await?;
                let is_live = detail.streams.iter().any(|s| s.status == "live");
                println!("  {} {}", theme::heading("Status:"), theme::live_status(is_live));

                println!("  {}", theme::heading("Recent streams:"));
                if detail.streams.is_empty() {
                    println!("    {}", theme::muted("(none)"));
                } else {
                    for s in detail.streams.iter().take(limit) {
                        let thumbnail = s.thumbnail_url.as_deref().filter(|u| !u.is_empty());
                        match thumbnail {
                            Some(url) => match print_stream_thumbnail(url, 6).await {
                                Ok((w, h)) => {
                                    execute!(stdout(), MoveRight((2 + w + 1) as u16))?;
                                    println!(
                                        "{} {} - {}",
                                        theme::status_tag(&s.status),
                                        s.title,
                                        theme::url(&s.url)
                                    );
                                    if h > 1 {
                                        execute!(stdout(), MoveDown((h - 1) as u16), MoveToColumn(0))?;
                                    }
                                }
                                Err(err) => {
                                    eprintln!("    (could not render stream thumbnail: {err})");
                                    println!(
                                        "    {} {} - {}",
                                        theme::status_tag(&s.status),
                                        s.title,
                                        theme::url(&s.url)
                                    );
                                }
                            },
                            None => println!(
                                "    {} {} - {}",
                                theme::status_tag(&s.status),
                                s.title,
                                theme::url(&s.url)
                            ),
                        }
                    }
                }

                println!("  {}", theme::heading("Recent clips:"));
                if detail.clips.is_empty() {
                    println!("    {}", theme::muted("(none)"));
                } else {
                    for c in detail.clips.iter().take(limit) {
                        println!(
                            "    {} ({}) - {}",
                            c.title,
                            theme::muted(&format!("{} views", c.view_count)),
                            theme::url(&c.url)
                        );
                    }
                }

                println!();
            }
        }
        Commands::Delete { name } => match delete_vtuber_channel(&name).await {
            Ok(v) => println!(
                "Successfully deleted VTuber channel name: {} with ID {}",
                v.english_name, v.id
            ),
            Err(err) => println!("Failed to delete VTuber channel '{name}': {err}"),
        },
        Commands::Sync { name } => match sync_vtuber_channels(Some(&name)).await {
            Ok(()) => println!("Successfully synced VTuber channel: {name}"),
            Err(err) => println!("Failed to sync VTuber channel '{name}': {err}"),
        },
        Commands::Live => {
            let live = fetch_live_vtubers().await?;
            if live.is_empty() {
                println!("{}", theme::muted("No VTuber is currently live"));
            }

            for entry in &live {
                let thumbnail = entry
                    .stream
                    .thumbnail_url
                    .as_deref()
                    .filter(|u| !u.is_empty());

                // Every entry here is by definition live, so the tag is
                // constant — it's kept for a consistent scan line with Lookup.
                let headline = format!(
                    "{} {} - {}",
                    theme::status_tag("live"),
                    theme::name(&entry.vtuber.english_name),
                    entry.stream.title
                );

                match thumbnail {
                    Some(url) => match print_stream_thumbnail(url, 6).await {
                        Ok((w, h)) => {
                            execute!(stdout(), MoveRight((2 + w + 1) as u16))?;
                            println!("{headline}");
                            if h > 1 {
                                execute!(stdout(), MoveDown((h - 1) as u16), MoveToColumn(0))?;
                            }
                        }
                        Err(err) => {
                            eprintln!("  (could not render stream thumbnail: {err})");
                            println!("{headline}");
                        }
                    },
                    None => println!("{headline}"),
                }

                println!("  {}", theme::url(&entry.stream.url));
                println!();
            }
        }
        Commands::Watch {
            interval,
            notify_existing,
        } => {
            watch::run(watch::WatchOptions {
                interval_secs: interval,
                notify_existing,
            })
            .await?;
        }
        Commands::Tui => tui::run().await?,
        Commands::Config => {
            let cfg = config::config();
            println!("Backend URL: {}", cfg.api_url);
            match &cfg.source {
                config::ConfigSource::Env => {
                    println!("Source:      OSHIHUB_API_URL environment variable")
                }
                config::ConfigSource::File(path) => println!("Source:      {}", path.display()),
                config::ConfigSource::Default => {
                    // Deliberately doesn't claim "no config file exists" — this
                    // branch is also reached when one exists but is empty or
                    // failed to parse (a warning above says which).
                    println!("Source:      built-in default");
                    if let Some(path) = config::config_path() {
                        println!("\nTo point at another backend, either set OSHIHUB_API_URL");
                        println!("or create {} containing:", path.display());
                        println!("\n    api_url = \"http://your-host:3000\"");
                        println!("    api_token = \"<shared secret>\"");
                    }
                }
            }

            // Never print the token itself — just whether one is set and
            // where it came from, which is all that's needed to debug a 401.
            match &cfg.token_source {
                Some(config::ConfigSource::Env) => {
                    println!("Auth token:  set (OSHIHUB_API_TOKEN environment variable)")
                }
                Some(config::ConfigSource::File(path)) => {
                    println!("Auth token:  set ({})", path.display())
                }
                Some(config::ConfigSource::Default) | None => {
                    println!("Auth token:  not set (fine for a local backend without API_TOKEN)")
                }
            }
        }
    }

    Ok(())
}
