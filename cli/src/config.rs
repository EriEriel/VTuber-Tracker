// Backend URL resolution.
//
// Every request function used to hardcode http://localhost:3000, which meant
// the CLI could only ever talk to a backend running on the same machine.
// Resolution order is env var > config file > built-in default, so an
// existing local setup keeps working with no config file present at all.

use serde::Deserialize;
use std::path::PathBuf;
use std::sync::OnceLock;

const DEFAULT_API_URL: &str = "http://localhost:3000";
const ENV_API_URL: &str = "OSHIHUB_API_URL";
const ENV_API_TOKEN: &str = "OSHIHUB_API_TOKEN";
const ENV_WATCH_INTERVAL: &str = "OSHIHUB_WATCH_INTERVAL";

/// Default gap between `oshihub watch` polls.
pub const DEFAULT_WATCH_INTERVAL_SECS: u64 = 60;
/// Floor on the poll interval. The backend detects a YouTube go-live on a
/// 5-minute cycle, so polling faster than this buys nothing and just adds
/// load — the limit is upstream, not here.
pub const MIN_WATCH_INTERVAL_SECS: u64 = 15;
/// mako's default-timeout is 5s, too short for "you weren't at your desk".
pub const DEFAULT_NOTIFY_TIMEOUT_MS: u64 = 10_000;

// Only the fields we actually read. Every key is optional so a config file
// that sets one setting doesn't have to spell out the rest.
#[derive(Deserialize)]
struct FileConfig {
    api_url: Option<String>,
    api_token: Option<String>,
    watch_interval_secs: Option<u64>,
    notify_icons: Option<bool>,
    notify_timeout_ms: Option<u64>,
}

// Where the resolved value came from — worth tracking so `oshihub config`
// can answer "why is it pointing there?" rather than just showing a URL.
#[derive(Debug, PartialEq)]
pub enum ConfigSource {
    Env,
    File(PathBuf),
    Default,
}

pub struct Config {
    pub api_url: String,
    pub source: ConfigSource,
    /// Shared secret sent as `Authorization: Bearer ...`. `None` when the
    /// backend is unauthenticated (i.e. a local one with no API_TOKEN set).
    pub api_token: Option<String>,
    pub token_source: Option<ConfigSource>,
    /// `oshihub watch` poll gap. A `--interval` flag overrides this.
    pub watch_interval_secs: u64,
    /// Whether `oshihub watch` caches VTuber avatars to use as notification
    /// icons. Opting out keeps the CLI from writing to disk at all.
    pub notify_icons: bool,
    /// How long a notification stays on screen. 0 means "until dismissed".
    pub notify_timeout_ms: u64,
}

/// `~/.config/oshihub/config.toml` on Linux (respects `XDG_CONFIG_HOME`);
/// `dirs` maps this to the right place on macOS/Windows too.
pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("oshihub").join("config.toml"))
}

// A trailing slash would produce "http://host//api/vtubers" once a path is
// appended, which some routers treat as a different (404ing) path.
fn normalize(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

// Treats unset and set-but-blank the same, so `OSHIHUB_API_TOKEN=` in a
// shell doesn't read as "the token is an empty string".
fn env_value(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Poll interval from the config sources, floored.
///
/// Split out and pure so the precedence chain and the clamp are testable
/// without touching the environment or the filesystem — `Config::load()`
/// itself isn't testable for exactly that reason. `--interval` is applied on
/// top of this by `watch`, since a flag beats any stored setting.
pub fn resolve_interval(env: Option<u64>, file: Option<u64>) -> u64 {
    env.or(file)
        .unwrap_or(DEFAULT_WATCH_INTERVAL_SECS)
        .max(MIN_WATCH_INTERVAL_SECS)
}

// Warns rather than returning an error: a typo'd config file shouldn't make
// every command unusable when a working default exists.
fn load_file() -> Option<(FileConfig, PathBuf)> {
    let path = config_path()?;
    let text = std::fs::read_to_string(&path).ok()?;
    match toml::from_str::<FileConfig>(&text) {
        Ok(parsed) => Some((parsed, path)),
        Err(err) => {
            eprintln!("Warning: ignoring invalid config at {}: {err}", path.display());
            None
        }
    }
}

impl Config {
    fn load() -> Self {
        let file = load_file();

        // Each setting resolves env > file > fallback independently, so the
        // URL can come from a config file while the token comes from the
        // environment (handy for keeping the secret out of a file on disk).
        let (api_url, source) = env_value(ENV_API_URL)
            .map(|url| (normalize(&url), ConfigSource::Env))
            .or_else(|| {
                let (cfg, path) = file.as_ref()?;
                let url = cfg.api_url.as_ref().filter(|u| !u.trim().is_empty())?;
                Some((normalize(url), ConfigSource::File(path.clone())))
            })
            .unwrap_or_else(|| (DEFAULT_API_URL.to_string(), ConfigSource::Default));

        let (api_token, token_source) = match env_value(ENV_API_TOKEN) {
            Some(token) => (Some(token), Some(ConfigSource::Env)),
            None => match file.as_ref().and_then(|(cfg, path)| {
                let token = cfg.api_token.as_ref().map(|t| t.trim())?;
                (!token.is_empty()).then(|| (token.to_string(), path.clone()))
            }) {
                Some((token, path)) => (Some(token), Some(ConfigSource::File(path))),
                None => (None, None),
            },
        };

        let watch_interval_secs = resolve_interval(
            env_value(ENV_WATCH_INTERVAL).and_then(|v| v.parse().ok()),
            file.as_ref().and_then(|(cfg, _)| cfg.watch_interval_secs),
        );

        Config {
            api_url,
            source,
            api_token,
            token_source,
            watch_interval_secs,
            notify_icons: file
                .as_ref()
                .and_then(|(cfg, _)| cfg.notify_icons)
                .unwrap_or(true),
            notify_timeout_ms: file
                .as_ref()
                .and_then(|(cfg, _)| cfg.notify_timeout_ms)
                .unwrap_or(DEFAULT_NOTIFY_TIMEOUT_MS),
        }
    }
}

// Resolved once per process. OnceLock is std's answer to lazy_static here —
// no crate needed, and the work happens on first use rather than at startup.
static CONFIG: OnceLock<Config> = OnceLock::new();

pub fn config() -> &'static Config {
    CONFIG.get_or_init(Config::load)
}

/// Base URL for the backend, without a trailing slash.
pub fn api_url() -> &'static str {
    &config().api_url
}

static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Shared HTTP client, carrying the bearer token as a default header so no
/// individual request has to remember it.
///
/// Also a genuine efficiency fix: `reqwest::Client` owns a connection pool
/// and is designed to be created once and reused. Every request function
/// used to build its own, which threw away pooling and TLS session reuse on
/// each call.
pub fn client() -> &'static reqwest::Client {
    CLIENT.get_or_init(|| {
        let mut headers = reqwest::header::HeaderMap::new();

        if let Some(token) = &config().api_token {
            match reqwest::header::HeaderValue::from_str(&format!("Bearer {token}")) {
                Ok(mut value) => {
                    // Keeps the token out of reqwest's Debug output, which is
                    // what gets printed when an error bubbles out of main().
                    value.set_sensitive(true);
                    headers.insert(reqwest::header::AUTHORIZATION, value);
                }
                Err(_) => eprintln!(
                    "Warning: api_token contains characters that can't be sent in a header; \
                     requests will be unauthenticated"
                ),
            }
        }

        reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

static EXTERNAL_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Client for requests to third parties — currently thumbnail images on
/// Twitch/YouTube CDNs.
///
/// Deliberately separate from `client()`: that one attaches the bearer token
/// as a *default* header, which would send our backend's shared secret to
/// every CDN host we fetch an image from. Tokens go to our own backend only.
pub fn external_client() -> &'static reqwest::Client {
    EXTERNAL_CLIENT.get_or_init(reqwest::Client::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_trailing_slashes() {
        assert_eq!(normalize("http://example.com:3000/"), "http://example.com:3000");
        assert_eq!(normalize("  http://example.com/  "), "http://example.com");
        assert_eq!(normalize("http://example.com"), "http://example.com");
    }

    #[test]
    fn parses_config_file_with_api_url() {
        let parsed: FileConfig = toml::from_str(r#"api_url = "http://vps.example.com:3000""#).unwrap();
        assert_eq!(parsed.api_url.as_deref(), Some("http://vps.example.com:3000"));
    }

    // An empty or unrelated config file should leave api_url unset rather
    // than failing to parse, so the default still applies.
    #[test]
    fn parses_config_file_without_api_url() {
        let parsed: FileConfig = toml::from_str("").unwrap();
        assert_eq!(parsed.api_url, None);
    }

    #[test]
    fn parses_watch_config_keys() {
        let parsed: FileConfig = toml::from_str(
            r#"
            api_url = "http://vps.example.com:3000"
            watch_interval_secs = 120
            notify_icons = false
            notify_timeout_ms = 0
        "#,
        )
        .unwrap();
        assert_eq!(parsed.watch_interval_secs, Some(120));
        assert_eq!(parsed.notify_icons, Some(false));
        assert_eq!(parsed.notify_timeout_ms, Some(0));
    }

    // A config written before these keys existed must still parse — the
    // watch settings just fall back to their defaults.
    #[test]
    fn watch_config_keys_are_optional() {
        let parsed: FileConfig = toml::from_str(r#"api_url = "http://localhost:3000""#).unwrap();
        assert_eq!(parsed.watch_interval_secs, None);
        assert_eq!(parsed.notify_icons, None);
        assert_eq!(parsed.notify_timeout_ms, None);
    }

    #[test]
    fn interval_resolves_env_over_file_over_default() {
        assert_eq!(resolve_interval(Some(90), Some(120)), 90);
        assert_eq!(resolve_interval(None, Some(120)), 120);
        assert_eq!(resolve_interval(None, None), DEFAULT_WATCH_INTERVAL_SECS);
    }

    // Polling faster than the floor buys nothing — the backend only detects
    // a YouTube go-live every 5 minutes — so an over-eager value is clamped
    // rather than honoured.
    #[test]
    fn interval_is_clamped_to_the_floor() {
        assert_eq!(resolve_interval(Some(1), None), MIN_WATCH_INTERVAL_SECS);
        assert_eq!(resolve_interval(None, Some(0)), MIN_WATCH_INTERVAL_SECS);
    }
}
