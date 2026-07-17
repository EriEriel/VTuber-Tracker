use crate::models::{Platform, VtuberChannel};
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
