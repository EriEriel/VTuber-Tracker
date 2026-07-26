use std::collections::HashSet;

use ratatui::widgets::ListState;

use crate::config::{self, ConfigSource};
use crate::models::{Platform, VtuberChannel};
use crate::routes::{LiveEntry, VtuberDetail};

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

/// Which of Detail's two sub-lists `j`/`k`/`o` act on. No third "nothing
/// focused" state — the header isn't interactive, so focus is always on one
/// of the two lists, even an empty one (navigation and `o` just no-op there).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailFocus {
    Streams,
    Clips,
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
    pub detail_focus: DetailFocus,
    pub stream_state: ListState,
    pub clip_state: ListState,
    /// Ids of currently-live VTubers, from the last `fetch_live_vtubers`.
    /// Empty until that fetch resolves — badges just don't show yet, same as
    /// the rest of the list before its own fetch resolves.
    pub live_ids: HashSet<String>,
    /// `L`'s toggle. When set, `visible_ids` narrows to `live_ids` members —
    /// nothing else needs to know about the filter, since selection and
    /// rendering both already go through `visible_ids`.
    pub live_only: bool,
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
            detail_focus: DetailFocus::Streams,
            stream_state: ListState::default(),
            clip_state: ListState::default(),
            live_ids: HashSet::new(),
            live_only: false,
            last_error: None,
        }
    }

    pub fn toggle_help(&mut self) {
        self.screen = match self.screen {
            Screen::Help => Screen::List,
            _ => Screen::Help,
        };
    }

    /// Indices into `items` that should actually render/be navigable right
    /// now — every row unfiltered, or only the live ones when `live_only` is
    /// set. `list_state`'s selected index is always a position *into this*,
    /// not into `items` directly, which is what lets Phase 4's incremental
    /// filter later reuse the same indirection for free text matching.
    pub fn visible_ids(&self) -> Vec<usize> {
        if !self.live_only {
            return (0..self.items.len()).collect();
        }
        self.items
            .iter()
            .enumerate()
            .filter(|(_, row)| self.live_ids.contains(&row.id))
            .map(|(i, _)| i)
            .collect()
    }

    pub fn selected(&self) -> Option<&VtuberRow> {
        let visible = self.visible_ids();
        let i = self.list_state.selected()?;
        visible.get(i).and_then(|&idx| self.items.get(idx))
    }

    /// Snaps `list_state` back into range after the visible set changes size
    /// out from under it (toggling the live filter, or the live set itself
    /// updating) — an out-of-range index isn't unsafe (ratatui just renders
    /// nothing highlighted), but leaving it stale is a needless UX papercut
    /// `cycle()` would otherwise only fix on the *next* keypress.
    fn ensure_selection_valid(&mut self) {
        let len = self.visible_ids().len();
        let still_valid = matches!(self.list_state.selected(), Some(i) if i < len);
        if !still_valid {
            self.list_state.select(if len > 0 { Some(0) } else { None });
        }
    }

    pub fn toggle_live_only(&mut self) {
        self.live_only = !self.live_only;
        self.ensure_selection_valid();
    }

    pub fn set_live(&mut self, entries: Vec<LiveEntry>) {
        self.live_ids = entries.into_iter().map(|e| e.vtuber.id).collect();
        if self.live_only {
            self.ensure_selection_valid();
        }
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
        self.detail_focus = DetailFocus::Streams;
        self.stream_state = ListState::default();
        self.clip_state = ListState::default();
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
                if !detail.streams.is_empty() {
                    self.stream_state.select(Some(0));
                }
                if !detail.clips.is_empty() {
                    self.clip_state.select(Some(0));
                }
                self.detail = Some(detail);
                self.detail_load = LoadState::Loaded;
            }
            Err(e) => self.detail_load = LoadState::Failed(e.to_string()),
        }
    }

    pub fn toggle_detail_focus(&mut self) {
        self.detail_focus = match self.detail_focus {
            DetailFocus::Streams => DetailFocus::Clips,
            DetailFocus::Clips => DetailFocus::Streams,
        };
    }

    pub fn detail_next(&mut self) {
        let len = self.focused_len();
        match self.detail_focus {
            DetailFocus::Streams => cycle(&mut self.stream_state, len, true),
            DetailFocus::Clips => cycle(&mut self.clip_state, len, true),
        }
    }

    pub fn detail_previous(&mut self) {
        let len = self.focused_len();
        match self.detail_focus {
            DetailFocus::Streams => cycle(&mut self.stream_state, len, false),
            DetailFocus::Clips => cycle(&mut self.clip_state, len, false),
        }
    }

    fn focused_len(&self) -> usize {
        let Some(detail) = &self.detail else { return 0 };
        match self.detail_focus {
            DetailFocus::Streams => detail.streams.len(),
            DetailFocus::Clips => detail.clips.len(),
        }
    }

    /// The URL of whichever stream/clip currently has focus in Detail, for
    /// `o` to open. `None` when there's nothing loaded yet or the focused
    /// list is empty — `event.rs` treats that as "nothing to do".
    pub fn focused_url(&self) -> Option<String> {
        let detail = self.detail.as_ref()?;
        match self.detail_focus {
            DetailFocus::Streams => {
                let i = self.stream_state.selected()?;
                detail.streams.get(i).map(|s| s.url.clone())
            }
            DetailFocus::Clips => {
                let i = self.clip_state.selected()?;
                detail.clips.get(i).map(|c| c.url.clone())
            }
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
        let len = self.visible_ids().len();
        cycle(&mut self.list_state, len, true);
    }

    pub fn previous(&mut self) {
        let len = self.visible_ids().len();
        cycle(&mut self.list_state, len, false);
    }
}

/// Shared wrap-around selection logic for every `ListState` this app has —
/// the main list plus Detail's streams/clips panes. A no-op on an empty
/// list, since three of the four call sites need to handle that.
fn cycle(state: &mut ListState, len: usize, forward: bool) {
    if len == 0 {
        return;
    }
    let i = match (state.selected(), forward) {
        (Some(i), true) if i + 1 < len => i + 1,
        (Some(_), true) => 0, // wrap forward
        (Some(0), false) => len - 1, // wrap backward
        (Some(i), false) => i - 1,
        (None, _) => 0,
    };
    state.select(Some(i));
}

fn describe_source(source: &ConfigSource) -> String {
    match source {
        ConfigSource::Env => "environment variable".to_string(),
        ConfigSource::File(path) => path.display().to_string(),
        ConfigSource::Default => "built-in default".to_string(),
    }
}
