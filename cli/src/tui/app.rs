use ratatui::widgets::ListState;

use crate::models::{Platform, VtuberChannel};

/// Minimal shape for what the list screen needs.
/// Map existing API DTO into this at the boundary (same mapper-at-the-
/// boundary pattern you used on the Hono backend) — don't let the raw API
/// response leak into the render layer.
#[derive(Debug, Clone)]
pub struct VtuberRow {
    pub name: String,
    pub platform: String, // "YouTube" | "Twitch" | "Holodex" etc.
}

/// Takes `&VtuberChannel` rather than owning one: the caller already has a
/// `Vec` from `routes`, and this only needs two fields out of ~15.
///
/// The display casing ("YouTube", not the wire's lowercase "youtube") is
/// decided *here*, not in `models`, because it's a presentation choice —
/// same reason `theme.rs` owns colour rather than each call site.
impl From<&VtuberChannel> for VtuberRow {
    fn from(channel: &VtuberChannel) -> Self {
        // `englishName` is the display name everywhere else in the CLI, but
        // the backend leaves it as an empty string for some Twitch channels
        // rather than omitting it, so fall back to the native `name`.
        let name = if channel.english_name.is_empty() {
            channel.name.clone()
        } else {
            channel.english_name.clone()
        };

        Self {
            name,
            platform: match channel.platform {
                Platform::Youtube => "YouTube".to_string(),
                Platform::Twitch => "Twitch".to_string(),
            },
        }
    }
}

pub enum LoadState {
    Loading,
    Loaded,
    Failed(String),
}

pub struct App {
    pub items: Vec<VtuberRow>,
    pub list_state: ListState,
    pub load_state: LoadState,
    pub should_quit: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            list_state: ListState::default(),
            load_state: LoadState::Loading,
            should_quit: false,
        }
    }

    pub fn set_items(&mut self, items: Vec<VtuberRow>) {
        self.load_state = LoadState::Loaded;
        if !items.is_empty() {
            self.list_state.select(Some(0));
        }
        self.items = items;
    }

    pub fn set_error(&mut self, msg: String) {
        self.load_state = LoadState::Failed(msg);
    }

    pub fn next(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) if i + 1 < self.items.len() => i + 1,
            Some(_) => 0, // wrap
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    pub fn previous(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(0) => self.items.len() - 1, // wrap
            Some(i) => i - 1,
            None => 0,
        };
        self.list_state.select(Some(i));
    }
}
