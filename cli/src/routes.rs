use std::option::Option;

use crate::models::{Platform, VtuberChannel,Source};
use serde::{Deserialize, Serialize};

pub async fn fetch_vtubers() -> Result<Vec<VtuberChannel>, Box<dyn std::error::Error>> {
    let client = reqwest::Client::new(); 
    let res = client
        .get("http://localhost:3000/api/vtubers")
        .send()
        .await?
        .text()
        .await?;

    let vtubers: Vec<VtuberChannel> = serde_json::from_str(&res)?;
    Ok(vtubers)
}


#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateVtuberChannel {
    platform: Platform,
    channel_id: String,
}

fn parse_channel_url(url: &str) -> Result<(Platform, String), Box<dyn std::error::Error>> {
    let without_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let without_www = without_scheme.strip_prefix("www.").unwrap_or(without_scheme);
    let (host, path) = without_www
        .split_once('/')
        .ok_or("URL is missing a channel path")?;

    let platform = if host.ends_with("youtube.com") || host.ends_with("youtu.be") {
        Platform::Youtube
    } else if host.ends_with("twitch.tv") {
        Platform::Twitch
    } else {
        return Err(format!("unsupported platform host: {host}").into());
    };

    let platform_channel_id = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|segment| !segment.is_empty())
        .ok_or("could not find a channel id in the URL")?
        .to_string();

    Ok((platform, platform_channel_id))
}

pub async fn create_vtuber_channel(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (platform, channel_id) = parse_channel_url(url)?;
    let body = CreateVtuberChannel {
        platform,
        channel_id,
    };

    let client = reqwest::Client::new();
    let res = client
        .post("http://localhost:3000/api/vtubers")
        .json(&body)
        .send()
        .await?;

    let status = res.status();
    let body = res.text().await?;

    if status.is_success() {
        println!("Successfully created VTuber channel from {url}");
    } else {
        println!("Failed to create VTuber channel from {url}. Status: {status}\n{body}");
    }

    Ok(())
}

pub async fn delete_vtuber_channel(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let vtubers = lookup_by_name(name).await?;
    let v = vtubers
        .first()
        .ok_or_else(|| format!("No VTuber matching '{}' found", name))?;

    let client = reqwest::Client::new();
    let res = client
        .delete(format!("http://localhost:3000/api/vtubers/{}", v.id))
        .send()
        .await?;

    let status = res.status();
    let body = res.text().await?;

    if status.is_success() {
        println!("Successfully deleted VTuber channel name: {} with ID {}", v.english_name, v.id);
    } else {
        println!("Failed to delete VTuber channel with ID {}. Status: {status}\n{body}", v.id);
    }

    Ok(())
}

pub async fn lookup_by_name(name: &str) -> Result<Vec<VtuberChannel>, Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let res = client
        .get("http://localhost:3000/api/vtubers")
        .query(&[("name", name)])
        .send()
        .await?
        .text()
        .await?;

    let vtubers: Vec<VtuberChannel> = serde_json::from_str(&res)?;
    Ok(vtubers)
}

#[derive(Deserialize)]
struct ProfileUrlResponse {
    url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamInfo {
    pub title: String,
    pub status: String,
    pub url: String,
    // thumbnailUrl is optional/nullable/empty-string on the backend schema —
    // not every stream (e.g. some sources, or a stream synced before this
    // field existed) is guaranteed to have one.
    #[serde(default)]
    pub thumbnail_url: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipInfo {
    pub title: String,
    pub url: String,
    pub view_count: u64,
}

#[derive(Deserialize)]
pub struct VtuberDetail {
    pub streams: Vec<StreamInfo>,
    pub clips: Vec<ClipInfo>,
}

// GET /api/vtubers/:id also returns `vtuber` and `snapshots`, but Lookup
// only needs streams/clips for now — serde ignores fields a struct doesn't
// declare, so there's no need to model the rest.
pub async fn fetch_vtuber_detail(id: &str) -> Result<VtuberDetail, Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let res = client
        .get(format!("http://localhost:3000/api/vtubers/{id}"))
        .send()
        .await?
        .text()
        .await?;

    let detail: VtuberDetail = serde_json::from_str(&res)?;
    Ok(detail)
}

// Fetches raw image bytes and renders them inline via viuer, which probes
// the terminal's actual capability (kitty protocol, iTerm's own protocol,
// Sixel, or a block-character fallback) rather than assuming kitty support.
// Decoding the bytes into a DynamicImage happens here because viuer's API
// takes an already-decoded image, not raw bytes — it only owns the
// terminal-output half, not fetching or decoding.
pub async fn print_thumbnail(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let bytes = client.get(url).send().await?.bytes().await?;
    let img = image::load_from_memory(&bytes)?;

    let conf = viuer::Config {
        width: Some(12),
        // viuer defaults to `absolute_offset: true`, which positions the
        // image at (x, y) relative to the terminal's top-left corner —
        // not the cursor. Without this, the image renders pinned to the
        // very top of the pane regardless of where Lookup's output
        // actually is, overlapping whatever's already there.
        absolute_offset: false,
        ..Default::default()
    };
    viuer::print(&img, &conf)?;
    Ok(())
}

// Same fetch/decode/render as print_thumbnail, but for placing a smaller
// image beside a line of text rather than stacked above it. `restore_cursor`
// snaps the cursor back to where it was *before* drawing (viuer saves that
// position internally, prior to applying `x`), so the caller can then move
// right by the returned (width, height) and print text on the same row,
// instead of the cursor landing below the image like print_thumbnail's does.
pub async fn print_stream_thumbnail(url: &str, width: u32) -> Result<(u32, u32), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let bytes = client.get(url).send().await?.bytes().await?;
    let img = image::load_from_memory(&bytes)?;

    let conf = viuer::Config {
        width: Some(width),
        x: 2,
        absolute_offset: false,
        restore_cursor: true,
        ..Default::default()
    };
    Ok(viuer::print(&img, &conf)?)
}

async fn fetch_profile_url(id: &str) -> Result<String, Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let res = client
        .get(format!("http://localhost:3000/api/vtubers/{id}/profile-url"))
        .send()
        .await?;

    let status = res.status();
    let body = res.text().await?;

    if !status.is_success() {
        return Err(format!("backend returned {status}: {body}").into());
    }

    let parsed: ProfileUrlResponse = serde_json::from_str(&body)?;
    Ok(parsed.url)
}

pub async fn jump_to(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let vtubers = lookup_by_name(name).await?;

    match vtubers.first() {
        Some(v) => match fetch_profile_url(&v.id).await {
            Ok(url) => {
                println!("Jumping to {} ({})", v.english_name, url);
                open::that(url)?;
            }
            Err(err) => println!("Could not resolve a channel URL for {}: {}", v.english_name, err),
        },
        None => println!("No VTuber matching '{}' found", name),
    }

    Ok(())
}

pub async fn sync_vtuber_channels(name: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    match name {
        Some(name) => {
            let vtubers = lookup_by_name(name).await?;
            match vtubers.first() {
                Some(v) => {
                    let sync_path = match v.source {
                        Source::Holodex => "holodex",
                        Source::Youtube_api => "youtube",
                        Source::Twitch_api => "twitch",
                    };

                    let client = reqwest::Client::new();
                    let res = client
                        .post(format!("http://localhost:3000/api/sync/{sync_path}?id={}&force=true", v.id))
                        .send()
                        .await?;

                    let status = res.status();
                    let body = res.text().await?;

                    if status.is_success() {
                        println!("Successfully synced VTuber channel: {}", v.english_name);
                    } else {
                        println!("Failed to sync VTuber channel: {}. Status: {status}\n{body}", v.english_name);
                    }
                }
                None => println!("VTuber channel {} is not founded in database.", name),
            }
        }
        None => {
                let client = reqwest::Client::new();
                let res = client
                    .post("http://localhost:3000/api/sync/all")
                    .body("force=true")
                    .send()
                    .await?;

                let status = res.status();
                let body = res.text().await?;

                if status.is_success() {
                    println!("Successfully synced all VTuber channels");
                } else {
                    println!("Failed to sync VTuber channels. Status: {status}\n{body}");
                }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_youtube_channel_url() {
        let (platform, id) =
            parse_channel_url("https://www.youtube.com/channel/UC1DCedRgGHBdm81E1llLhOQ").unwrap();
        assert_eq!(platform, Platform::Youtube);
        assert_eq!(id, "UC1DCedRgGHBdm81E1llLhOQ");
    }

    #[test]
    fn parses_youtube_handle_url() {
        let (platform, id) =
            parse_channel_url("https://www.youtube.com/@holoen_raorapanthera").unwrap();
        assert_eq!(platform, Platform::Youtube);
        assert_eq!(id, "@holoen_raorapanthera");
    }

    #[test]
    fn parses_twitch_channel_url() {
        let (platform, id) = parse_channel_url("https://twitch.tv/tawffie/").unwrap();
        assert_eq!(platform, Platform::Twitch);
        assert_eq!(id, "tawffie");
    }

    #[test]
    fn rejects_unsupported_platform() {
        assert!(parse_channel_url("https://example.com/foo").is_err());
    }
}
