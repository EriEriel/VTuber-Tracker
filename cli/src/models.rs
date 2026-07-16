use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VtuberChannel {
    #[serde(rename = "_id")]
    pub id: String,
    #[serde(rename = "__v")]
    pub version: u32,

    pub created_at: String,
    pub updated_at: String,
    pub last_synced_at: Option<String>,
    pub last_live_synced_at: Option<String>,
    pub last_stats_synced_at: Option<String>,

    pub english_name: String,
    pub is_tracked: bool,
    pub name: String,
    pub org: Option<String>,
    pub suborg: Option<String>,
    pub photo: String,

    pub platform: Platform,
    pub platform_channel_id: String,
    pub source: Source,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Youtube,
    Twitch,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[allow(non_camel_case_types)]
pub enum Source {
    Holodex,
    Youtube_api,
    Twitch_api,
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real response shape for a Holodex-sourced channel: has org/suborg.
    const HOLODEX_CHANNEL: &str = r#"{
        "_id": "6a3fa73271b0fd7edb4403e5",
        "name": "Pekora Ch. 兎田ぺこら",
        "englishName": "Usada Pekora",
        "photo": "https://yt3.ggpht.com/example.jpg",
        "platform": "youtube",
        "source": "holodex",
        "platformChannelId": "UC1DCedRgGHBdm81E1llLhOQ",
        "org": "Hololive",
        "suborg": "3rd Generation (Fantasy)",
        "isTracked": true,
        "lastSyncedAt": "2026-06-27T10:34:27.672Z",
        "lastLiveSyncedAt": "2026-06-27T10:34:27.672Z",
        "lastStatsSyncedAt": "2026-06-27T10:34:27.672Z",
        "createdAt": "2026-06-27T10:34:26.233Z",
        "updatedAt": "2026-06-27T10:34:32.757Z",
        "__v": 0
    }"#;

    // Real response shape for a Twitch-sourced channel: org/suborg are absent entirely.
    const TWITCH_CHANNEL: &str = r#"{
        "_id": "6a3fa73371b0fd7edb4403e6",
        "name": "Tawffie",
        "englishName": "Tawffie",
        "photo": "https://static-cdn.jtvnw.net/example.png",
        "platform": "twitch",
        "source": "twitch_api",
        "platformChannelId": "118858663",
        "isTracked": true,
        "lastSyncedAt": "2026-06-27T10:34:33.007Z",
        "lastLiveSyncedAt": "2026-06-27T10:34:33.007Z",
        "lastStatsSyncedAt": "2026-06-27T10:34:33.007Z",
        "createdAt": "2026-06-27T10:34:27.587Z",
        "updatedAt": "2026-06-27T10:34:37.671Z",
        "__v": 0
    }"#;

    #[test]
    fn deserializes_channel_with_org_and_suborg() {
        let channel: VtuberChannel = serde_json::from_str(HOLODEX_CHANNEL).unwrap();

        assert_eq!(channel.id, "6a3fa73271b0fd7edb4403e5");
        assert_eq!(channel.english_name, "Usada Pekora");
        assert_eq!(channel.platform, Platform::Youtube);
        assert_eq!(channel.source, Source::Holodex);
        assert_eq!(channel.org.as_deref(), Some("Hololive"));
        assert_eq!(channel.suborg.as_deref(), Some("3rd Generation (Fantasy)"));
    }

    #[test]
    fn deserializes_channel_missing_org_and_suborg() {
        let channel: VtuberChannel = serde_json::from_str(TWITCH_CHANNEL).unwrap();

        assert_eq!(channel.platform, Platform::Twitch);
        assert_eq!(channel.source, Source::Twitch_api);
        assert_eq!(channel.org, None);
        assert_eq!(channel.suborg, None);
    }

    #[test]
    fn deserializes_array_response() {
        let body = format!("[{HOLODEX_CHANNEL},{TWITCH_CHANNEL}]");
        let channels: Vec<VtuberChannel> = serde_json::from_str(&body).unwrap();

        assert_eq!(channels.len(), 2);
    }
}
