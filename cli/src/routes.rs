use std::option::Option;

use crate::config::api_url;
use crate::models::{Platform, VtuberChannel,Source};
use serde::{Deserialize, Serialize};

// A typed error, rather than the `Box<dyn Error>` built from a string that
// every other path here uses.
//
// `oshihub watch` runs for hours and has to treat failures differently: a
// dropped connection is transient and should be retried, but a 401 is fatal
// (no amount of retrying fixes a bad token, and a watcher silently 401ing
// forever while you believe you're covered is the worst possible state).
// Distinguishing those by matching on a variant beats substring-matching an
// error message.
//
// Existing `Box<dyn Error>` call sites are unaffected — `?` still converts
// via the blanket `From<E: Error>` impl.
#[derive(Debug)]
pub enum ApiError {
    Unauthorized,
    Http {
        status: reqwest::StatusCode,
        body: String,
    },
    Transport(reqwest::Error),
    Decode(serde_json::Error),
    // Local validation failures that never reach the network — a malformed
    // URL, a name that matched nothing. Kept on this enum rather than a
    // second error type so every routes.rs function still returns one thing.
    Invalid(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Unauthorized => write!(
                f,
                "backend rejected the request (401 Unauthorized) — check the auth token with `oshihub config`"
            ),
            ApiError::Http { status, body } => write!(f, "backend returned {status}: {body}"),
            // `{err}` not `{err:?}`: reqwest's Debug is a multi-line dump of
            // the whole request context, which is what made an unreachable
            // backend look like a crash rather than "couldn't connect".
            ApiError::Transport(err) => write!(f, "could not reach the backend: {err}"),
            ApiError::Decode(err) => write!(f, "could not parse the backend's response: {err}"),
            ApiError::Invalid(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ApiError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ApiError::Transport(err) => Some(err),
            ApiError::Decode(err) => Some(err),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for ApiError {
    fn from(err: reqwest::Error) -> Self {
        ApiError::Transport(err)
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(err: serde_json::Error) -> Self {
        ApiError::Decode(err)
    }
}

// Reads a response body, turning a non-2xx status into a readable error.
//
// Without this the error-shaped JSON body of a failure ({"error": "..."})
// gets handed straight to serde, which fails with something like
// `invalid type: map, expected a sequence` — technically true and completely
// unhelpful for the actual problem, which is usually a missing token.
async fn read_body(res: reqwest::Response) -> Result<String, ApiError> {
    let status = res.status();
    let body = res.text().await?;

    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(ApiError::Unauthorized);
    }
    if !status.is_success() {
        return Err(ApiError::Http { status, body });
    }

    Ok(body)
}

pub async fn fetch_vtubers() -> Result<Vec<VtuberChannel>, ApiError> {
    let client = crate::config::client();
    let res = client
        .get(format!("{}/api/vtubers", api_url()))
        .send()
        .await?;

    let body = read_body(res).await?;
    let vtubers: Vec<VtuberChannel> = serde_json::from_str(&body)?;
    Ok(vtubers)
}


#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateVtuberChannel {
    platform: Platform,
    channel_id: String,
}

pub(crate) fn parse_channel_url(url: &str) -> Result<(Platform, String), ApiError> {
    let without_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let without_www = without_scheme.strip_prefix("www.").unwrap_or(without_scheme);
    let (host, path) = without_www
        .split_once('/')
        .ok_or_else(|| ApiError::Invalid("URL is missing a channel path".to_string()))?;

    let platform = if host.ends_with("youtube.com") || host.ends_with("youtu.be") {
        Platform::Youtube
    } else if host.ends_with("twitch.tv") {
        Platform::Twitch
    } else {
        return Err(ApiError::Invalid(format!("unsupported platform host: {host}")));
    };

    let platform_channel_id = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|segment| !segment.is_empty())
        .ok_or_else(|| ApiError::Invalid("could not find a channel id in the URL".to_string()))?
        .to_string();

    Ok((platform, platform_channel_id))
}

pub async fn create_vtuber_channel(url: &str) -> Result<(), ApiError> {
    let (platform, channel_id) = parse_channel_url(url)?;
    let body = CreateVtuberChannel {
        platform,
        channel_id,
    };

    let client = crate::config::client();
    let res = client
        .post(format!("{}/api/vtubers", api_url()))
        .json(&body)
        .send()
        .await?;

    read_body(res).await?;
    Ok(())
}

/// Deletes by id directly — the TUI already has the exact `VtuberChannel`
/// selected, so it has no reason to go through a name lookup that could, in
/// principle, match a different record than the one on screen.
pub async fn delete_vtuber_channel_by_id(id: &str) -> Result<(), ApiError> {
    let client = crate::config::client();
    let res = client
        .delete(format!("{}/api/vtubers/{}", api_url(), id))
        .send()
        .await?;

    read_body(res).await?;
    Ok(())
}

pub async fn delete_vtuber_channel(name: &str) -> Result<VtuberChannel, ApiError> {
    let vtubers = lookup_by_name(name).await?;
    let v = vtubers
        .into_iter()
        .next()
        .ok_or_else(|| ApiError::Invalid(format!("No VTuber matching '{}' found", name)))?;

    delete_vtuber_channel_by_id(&v.id).await?;
    Ok(v)
}

pub async fn lookup_by_name(name: &str) -> Result<Vec<VtuberChannel>, ApiError> {
    let client = crate::config::client();
    let res = client
        .get(format!("{}/api/vtubers", api_url()))
        .query(&[("name", name)])
        .send()
        .await?;

    let body = read_body(res).await?;
    let vtubers: Vec<VtuberChannel> = serde_json::from_str(&body)?;
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
pub async fn fetch_vtuber_detail(id: &str) -> Result<VtuberDetail, ApiError> {
    let client = crate::config::client();
    let res = client
        .get(format!("{}/api/vtubers/{id}", api_url()))
        .send()
        .await?;

    let body = read_body(res).await?;
    let detail: VtuberDetail = serde_json::from_str(&body)?;
    Ok(detail)
}

// Fetches raw image bytes and renders them inline via viuer, which probes
// the terminal's actual capability (kitty protocol, iTerm's own protocol,
// Sixel, or a block-character fallback) rather than assuming kitty support.
// Decoding the bytes into a DynamicImage happens here because viuer's API
// takes an already-decoded image, not raw bytes — it only owns the
// terminal-output half, not fetching or decoding.
pub async fn print_thumbnail(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = crate::config::external_client();
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
    let client = crate::config::external_client();
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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveStreamInfo {
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub thumbnail_url: Option<String>,
    /// Stable per-stream identity, used by `oshihub watch` to tell one stream
    /// from the next. `Option` deliberately: a CLI updated before the backend
    /// is redeployed won't see this field, and degrading to a coarser key is
    /// better than failing to deserialize.
    #[serde(default)]
    pub external_id: Option<String>,
    /// Only used to order duplicate live docs deterministically. Left as a
    /// String — nothing here parses dates, and pulling in a date crate to
    /// render "live for 1h23m" isn't worth it yet.
    #[serde(default)]
    pub start_time: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LiveEntry {
    pub vtuber: VtuberChannel,
    pub stream: LiveStreamInfo,
}

pub async fn fetch_live_vtubers() -> Result<Vec<LiveEntry>, ApiError> {
    let client = crate::config::client();
    let res = client
        .get(format!("{}/api/vtubers/live", api_url()))
        .send()
        .await?;

    let body = read_body(res).await?;
    let live: Vec<LiveEntry> = serde_json::from_str(&body)?;
    Ok(live)
}

pub(crate) async fn fetch_profile_url(id: &str) -> Result<String, ApiError> {
    let client = crate::config::client();
    let res = client
        .get(format!("{}/api/vtubers/{id}/profile-url", api_url()))
        .send()
        .await?;

    let body = read_body(res).await?;
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

fn sync_path_for(source: Source) -> &'static str {
    match source {
        Source::Holodex => "holodex",
        Source::Youtube_api => "youtube",
        Source::Twitch_api => "twitch",
    }
}

/// Syncs by id directly, same reasoning as `delete_vtuber_channel_by_id` —
/// the TUI already knows exactly which record and which `source` it is.
pub async fn sync_vtuber_channel_by_id(id: &str, source: Source) -> Result<(), ApiError> {
    let sync_path = sync_path_for(source);
    let client = crate::config::client();
    let res = client
        .post(format!("{}/api/sync/{sync_path}?id={id}&force=true", api_url()))
        .send()
        .await?;

    read_body(res).await?;
    Ok(())
}

pub async fn sync_vtuber_channels(name: Option<&str>) -> Result<(), ApiError> {
    match name {
        Some(name) => {
            let vtubers = lookup_by_name(name).await?;
            let v = vtubers.into_iter().next().ok_or_else(|| {
                ApiError::Invalid(format!("VTuber channel {name} is not founded in database."))
            })?;

            sync_vtuber_channel_by_id(&v.id, v.source).await?;
        }
        None => {
            let client = crate::config::client();
            let res = client
                .post(format!("{}/api/sync/all", api_url()))
                .body("force=true")
                .send()
                .await?;

            read_body(res).await?;
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

    // Real captured `GET /api/vtubers/live` entry (Tawffie, live 2026-07-26).
    const LIVE_ENTRY_TWITCH: &str = r#"{"vtuber":{"_id":"6a65488f723226c982581293","name":"Tawffie","englishName":"Tawffie","photo":"https://static-cdn.jtvnw.net/jtv_user_pictures/30e185a0-profile_image-300x300.png","platform":"twitch","source":"twitch_api","platformChannelId":"118858663","isTracked":true,"lastSyncedAt":"2026-07-26T03:09:20.372Z","lastLiveSyncedAt":"2026-07-26T03:09:20.372Z","lastStatsSyncedAt":"2026-07-26T03:09:20.372Z","createdAt":"2026-07-25T23:36:47.273Z","updatedAt":"2026-07-26T03:09:41.433Z","__v":0},"stream":{"externalId":"317728559192","title":"First Playthrough.. Hay-deez...nuts? !throne !clip","url":"https://www.twitch.tv/tawffie","thumbnailUrl":"https://static-cdn.jtvnw.net/previews-ttv/live_user_tawffie-640x360.jpg","startTime":"2026-07-26T03:09:00.000Z"}}"#;

    // Same shape for a Holodex-sourced YouTube VTuber: the stream URL carries
    // the video id, unlike Twitch's.
    const LIVE_ENTRY_YOUTUBE: &str = r#"{"vtuber":{"_id":"6a64b8bb723226c982580e29","name":"Nimi Nightmare","englishName":"Nimi Nightmare","photo":"https://yt3.ggpht.com/nimi=s800-c-k-c0x00ffffff-no-rj","platform":"youtube","source":"holodex","platformChannelId":"UCIfAvpeIWGHb0duCkMkmm2Q","isTracked":true,"lastSyncedAt":"2026-07-26T03:09:20.372Z","lastLiveSyncedAt":"2026-07-26T03:09:20.372Z","lastStatsSyncedAt":"2026-07-26T03:09:20.372Z","createdAt":"2026-07-25T23:36:47.273Z","updatedAt":"2026-07-26T03:09:41.433Z","__v":0},"stream":{"externalId":"0iTYVln-O1Q","title":"karaoke stream","url":"https://www.youtube.com/watch?v=0iTYVln-O1Q","thumbnailUrl":"https://i.ytimg.com/vi/0iTYVln-O1Q/hqdefault.jpg","startTime":"2026-07-26T01:00:00.000Z"}}"#;

    // A backend predating the externalId addition. The CLI can be updated
    // before the VPS is redeployed, so this has to deserialize rather than
    // fail — `watch` falls back to a coarser dedup key.
    const LIVE_ENTRY_NO_EXTERNAL_ID: &str = r#"{"vtuber":{"_id":"6a65488f723226c982581293","name":"Tawffie","englishName":"Tawffie","photo":"","platform":"twitch","source":"twitch_api","platformChannelId":"118858663","isTracked":true,"lastSyncedAt":null,"lastLiveSyncedAt":null,"lastStatsSyncedAt":null,"createdAt":"2026-07-25T23:36:47.273Z","updatedAt":"2026-07-26T03:09:41.433Z","__v":0},"stream":{"title":"old backend","url":"https://www.twitch.tv/tawffie","thumbnailUrl":null}}"#;

    #[test]
    fn deserializes_twitch_live_entry() {
        let entry: LiveEntry = serde_json::from_str(LIVE_ENTRY_TWITCH).unwrap();
        assert_eq!(entry.vtuber.english_name, "Tawffie");
        assert_eq!(entry.stream.external_id.as_deref(), Some("317728559192"));
        // Regression guard: startTime is sent by the backend and was silently
        // dropped before this field existed.
        assert_eq!(
            entry.stream.start_time.as_deref(),
            Some("2026-07-26T03:09:00.000Z")
        );
    }

    #[test]
    fn deserializes_youtube_live_entry() {
        let entry: LiveEntry = serde_json::from_str(LIVE_ENTRY_YOUTUBE).unwrap();
        assert_eq!(entry.vtuber.platform, Platform::Youtube);
        assert_eq!(entry.stream.external_id.as_deref(), Some("0iTYVln-O1Q"));
    }

    #[test]
    fn deserializes_live_entry_without_external_id() {
        let entry: LiveEntry = serde_json::from_str(LIVE_ENTRY_NO_EXTERNAL_ID).unwrap();
        assert_eq!(entry.stream.external_id, None);
        assert_eq!(entry.stream.start_time, None);
        assert_eq!(entry.stream.thumbnail_url, None);
    }

    #[test]
    fn deserializes_live_array_response() {
        let body = format!("[{LIVE_ENTRY_TWITCH},{LIVE_ENTRY_YOUTUBE}]");
        let entries: Vec<LiveEntry> = serde_json::from_str(&body).unwrap();
        assert_eq!(entries.len(), 2);
    }
}
