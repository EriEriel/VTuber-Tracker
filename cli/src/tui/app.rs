use ratatui::widgets::ListState;

use crate::config::{self, ConfigSource};
use crate::models::{Platform, VtuberChannel};
use crate::routes::VtuberDetail;

/// Minimal shape for what the list *and* detail screens need.
/// Map existing API DTO into this at the boundary (same mapper-at-the-
/// boundary pattern you used on the Hono backend) — don't let the raw API
/// response leak into the render layer.
#[derive(Debug, Clone)]
pub struct VtuberRow {
    pub id: String,
    pub name: String,
    pub platform: String, // "YouTube" | "Twitch" | "Holodex" etc.
    pub org: Option<String>,
    pub suborg: Option<String>,
}

/// Takes `&VtuberChannel` rather than owning one: the caller already has a
/// `Vec` from `routes`, and this only needs a handful of fields out of ~15.
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
            id: channel.id.clone(),
            name,
            platform: match channel.platform {
                Platform::Youtube => "YouTube".to_string(),
                Platform::Twitch => "Twitch".to_string(),
            },
            org: channel.org.clone(),
            suborg: channel.suborg.clone(),
        }
    }
}

pub enum LoadState {
    Loading,
    Loaded,
    Failed(String),
}

/// Which screen is on top. `Help` overlays whatever `load_state` is
/// currently rendering underneath, rather than replacing it — closing the
/// overlay returns to exactly what was there before. `Detail` and `Help` are
/// both only ever reached from `List` and both return straight to it — no
/// stack, since nothing yet needs one (that's Phase 5's modals).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    List,
    Detail,
    Help,
}

pub struct App {
    pub items: Vec<VtuberRow>,
    pub list_state: ListState,
    pub load_state: LoadState,
    pub should_quit: bool,
    pub screen: Screen,
    /// Config summary for the `?` overlay, resolved once at startup —
    /// `ui::draw` stays a pure function of `&App`, so it reads these instead
    /// of calling back into `config::config()` itself.
    pub config_url: String,
    pub config_source: String,
    pub config_token: String,
    /// Streams/clips for whichever `VtuberRow` is currently selected. `None`
    /// until the background fetch resolves; `detail_load` tracks that fetch
    /// the same way `load_state` tracks the list's.
    pub detail: Option<VtuberDetail>,
    pub detail_load: LoadState,
    /// The id a detail fetch is in flight for. If the user backs out and
    /// opens a *different* VTuber before the first fetch resolves, the late
    /// response's id won't match this any more and gets discarded instead of
    /// overwriting the detail screen for the wrong VTuber.
    pending_detail_id: Option<String>,
    /// Last background-action failure (currently only `o`'s open-in-browser),
    /// shown in the status bar until the next action or screen change clears
    /// it. Unlike a list/detail load failure, an action failure has no
    /// screen of its own to replace, so it has to surface somewhere.
    pub last_error: Option<String>,
}

impl App {
    pub fn new() -> Self {
        let cfg = config::config();
        Self {
            items: Vec::new(),
            list_state: ListState::default(),
            load_state: LoadState::Loading,
            should_quit: false,
            screen: Screen::List,
            config_url: cfg.api_url.clone(),
            config_source: describe_source(&cfg.source),
            config_token: match &cfg.token_source {
                Some(source) => format!("set ({})", describe_source(source)),
                // Never the token itself — matching `Commands::Config` in
                // main.rs, this is only ever enough to debug a 401.
                None => "not set".to_string(),
            },
            detail: None,
            detail_load: LoadState::Loading,
            pending_detail_id: None,
            last_error: None,
        }
    }

    pub fn toggle_help(&mut self) {
        self.screen = match self.screen {
            Screen::Help => Screen::List,
            _ => Screen::Help,
        };
    }

    pub fn selected(&self) -> Option<&VtuberRow> {
        self.list_state.selected().and_then(|i| self.items.get(i))
    }

    pub fn go_back_to_list(&mut self) {
        self.screen = Screen::List;
        self.last_error = None;
    }

    pub fn begin_detail(&mut self, id: String) {
        self.screen = Screen::Detail;
        self.detail = None;
        self.detail_load = LoadState::Loading;
        self.pending_detail_id = Some(id);
        self.last_error = None;
    }

    /// Applies a detail fetch's result, unless it's for a VTuber the user
    /// has since navigated away from (see `pending_detail_id`).
    pub fn accept_detail(&mut self, id: &str, result: Result<VtuberDetail, crate::routes::ApiError>) {
        if self.pending_detail_id.as_deref() != Some(id) {
            return;
        }
        self.pending_detail_id = None;
        match result {
            Ok(detail) => {
                self.detail = Some(detail);
                self.detail_load = LoadState::Loaded;
            }
            Err(e) => self.detail_load = LoadState::Failed(e.to_string()),
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

fn describe_source(source: &ConfigSource) -> String {
    match source {
        ConfigSource::Env => "environment variable".to_string(),
        ConfigSource::File(path) => path.display().to_string(),
        ConfigSource::Default => "built-in default".to_string(),
    }
}
