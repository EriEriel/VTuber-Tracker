use std::collections::{HashMap, HashSet};

use ratatui::layout::Size;
use ratatui::widgets::ListState;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use ratatui_image::Resize;

use crate::config::{self, ConfigSource};
use crate::models::{Platform, Source, VtuberChannel};
use crate::routes::{LiveEntry, VtuberDetail};
use crate::watch::{self, WatchState};

/// The avatar's fixed render box, in terminal cells. `ui::draw_detail` must
/// split its header layout to exactly this size — `Picker::new_protocol`
/// bakes this size into the `Protocol` at creation time (see `accept_avatar`),
/// so rendering `Image::new(protocol)` into a differently-sized `Rect` later
/// would fit a stale box, not the one actually drawn into. One shared
/// constant instead of two files agreeing on `16`/`8` independently.
pub const AVATAR_COLS: u16 = 16;
pub const AVATAR_ROWS: u16 = 8;

/// Same idea as `AVATAR_COLS`/`AVATAR_ROWS`, sized wider rather than square —
/// stream thumbnails are landscape (YouTube/Twitch previews), not avatars.
pub const THUMB_COLS: u16 = 28;
pub const THUMB_ROWS: u16 = 8;

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
    /// Which upstream API to sync through — `s` needs this to pick the right
    /// `/api/sync/{path}` without a redundant name-based lookup.
    pub source: Source,
    /// Raw fields `name` above already collapsed away (it's `english_name`
    /// falling back to `native_name`) — `e`'s edit form needs both
    /// separately to prefill correctly, plus `photo`/`is_tracked`, which
    /// nothing before Phase 6 needed at all.
    pub english_name: String,
    pub native_name: String,
    pub photo: String,
    pub is_tracked: bool,
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
            source: channel.source,
            english_name: channel.english_name.clone(),
            native_name: channel.name.clone(),
            photo: channel.photo.clone(),
            is_tracked: channel.is_tracked,
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

/// Which screen is on top. `Help`/`Modal` overlay whatever `load_state` is
/// currently rendering underneath, rather than replacing it — closing them
/// returns to exactly what was there before. `Detail`/`Help`/`Modal` are
/// only ever reached from `List` and return straight to it. `Dashboard` is
/// the one exception: reachable from both `List` and `Detail`, so it
/// remembers its entry point in `App::dashboard_return` — still a single
/// slot, not a stack, since one level of "where did I come from" is all
/// anything needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    List,
    Detail,
    Dashboard,
    Help,
    Modal(ModalKind),
}

/// One tracked week, ready for a labelled `BarChart` bar.
pub struct WeekBar {
    /// The bucket's start date as "MM-DD" ("07-06"), sliced from the
    /// backend's "YYYY-MM-DD". Empty when the backend predates `starts`
    /// (see `routes::StreamFrequency`) — an unlabelled bar, not a crash.
    pub label: String,
    pub count: u64,
    /// The current, still-accumulating week — always the newest bar.
    /// Rendered dimmed and labelled "now" so its low count reads as "week
    /// just started", not "streaming stopped".
    pub partial: bool,
}

/// `g`'s per-VTuber stats view, mapped from `routes::StreamFrequency` at the
/// boundary (same rule as `VtuberRow`: the wire DTO must not leak into the
/// render layer). All the summary numbers are computed once here, on accept,
/// so `ui::draw` just prints fields.
pub struct FrequencyView {
    /// Tracked buckets only, oldest → newest. Pre-tracking (`None`) buckets
    /// are dropped here rather than charted as an "absent" region — a wall
    /// of no-data columns was the most visually dominant thing on the first
    /// cut of this chart, which is exactly backwards. The summary's `since`
    /// date carries that information instead. `ui.rs` renders the *tail* of
    /// this (newest weeks win) when the pane can't fit all of it.
    pub weeks: Vec<WeekBar>,
    /// Sum over all tracked buckets, including the current partial week.
    pub total: u64,
    /// The newest bucket — always the current, still-accumulating week, so
    /// it's labelled "this week" rather than folded into the average below.
    pub this_week: u64,
    /// Mean over *complete* tracked weeks (the current partial one excluded —
    /// it would drag the average down every Monday). `None` until at least
    /// one complete tracked week exists.
    pub avg_per_week: Option<f64>,
    pub peak: u64,
    /// `firstStreamAt` truncated to its date half, for "since 2026-07-10".
    pub since: Option<String>,
}

impl From<crate::routes::StreamFrequency> for FrequencyView {
    fn from(freq: crate::routes::StreamFrequency) -> Self {
        // Tracked buckets only, oldest → newest; `starts` is parallel to
        // `counts`, so the index survives the `None`-filtering to look up
        // each kept bucket's date.
        let weeks: Vec<WeekBar> = freq
            .counts
            .iter()
            .enumerate()
            .filter_map(|(i, c)| c.map(|count| (i, count)))
            .map(|(i, count)| WeekBar {
                // "YYYY-MM-DD" → "MM-DD". `chars().skip` rather than a byte
                // slice so a malformed short string degrades to empty
                // instead of panicking on an out-of-range index.
                label: freq
                    .starts
                    .get(i)
                    .map(|s| s.chars().skip(5).take(5).collect())
                    .unwrap_or_default(),
                count,
                // The newest *bucket* is always the current week; if it was
                // tracked it's the last thing `filter_map` kept.
                partial: i == freq.counts.len() - 1,
            })
            .collect();

        let total = weeks.iter().map(|w| w.count).sum();
        let this_week = weeks.last().map(|w| w.count).unwrap_or(0);
        let complete = weeks.len().saturating_sub(1);
        let avg_per_week = (complete > 0)
            .then(|| (total - this_week) as f64 / complete as f64);

        Self {
            total,
            this_week,
            avg_per_week,
            peak: weeks.iter().map(|w| w.count).max().unwrap_or(0),
            weeks,
            since: freq.first_stream_at.map(|s| s.chars().take(10).collect()),
        }
    }
}

/// `d` and `a` both need exclusive keyboard focus the same way `/` does —
/// `event.rs` short-circuits on `Screen::Modal(_)` before the normal keymap,
/// same reasoning as `filter_editing`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalKind {
    ConfirmDelete,
    CreateUrl,
    Edit,
}

/// Which field `e`'s edit form currently has keyboard focus. `Tab`/`Down`
/// advances, `BackTab`/`Up` retreats; wraps both ways so cycling never dead-
/// ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditField {
    Name,
    EnglishName,
    Photo,
    Org,
    Suborg,
    IsTracked,
}

impl EditField {
    fn next(self) -> Self {
        match self {
            EditField::Name => EditField::EnglishName,
            EditField::EnglishName => EditField::Photo,
            EditField::Photo => EditField::Org,
            EditField::Org => EditField::Suborg,
            EditField::Suborg => EditField::IsTracked,
            EditField::IsTracked => EditField::Name,
        }
    }

    fn previous(self) -> Self {
        match self {
            EditField::Name => EditField::IsTracked,
            EditField::EnglishName => EditField::Name,
            EditField::Photo => EditField::EnglishName,
            EditField::Org => EditField::Photo,
            EditField::Suborg => EditField::Org,
            EditField::IsTracked => EditField::Suborg,
        }
    }
}

/// `e`'s in-progress form, prefilled from the selected `VtuberRow` when
/// opened. Bundled into its own struct rather than flat fields on `App`
/// (unlike `create_input`) — six text fields plus a toggle, focus, and an
/// error is too much to spread across the top-level struct.
pub struct EditForm {
    pub id: String,
    /// The pre-edit display name, carried through for the "Updated X"
    /// status message — not itself editable (`name`/`english_name` below
    /// are the actual editable fields the message is derived from on submit).
    pub display_name: String,
    pub name: String,
    pub english_name: String,
    pub photo: String,
    pub org: String,
    pub suborg: String,
    pub is_tracked: bool,
    pub focus: EditField,
    pub error: Option<String>,
}

/// What `App::try_submit_edit` hands back for `event.rs` to wrap into a
/// `Command::Update` — plain data, not a `routes::UpdateFields`, so
/// `event.rs` stays routes-agnostic like every other `Command` payload.
pub struct EditPayload {
    pub id: String,
    pub display_name: String,
    pub name: String,
    pub english_name: String,
    pub photo: String,
    pub org: String,
    pub suborg: String,
    pub is_tracked: bool,
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
    /// Terminal graphics-protocol capability, probed once at startup via
    /// `Picker::from_query_stdio` — same "resolved once into an `App` field"
    /// pattern as `config_url` above, and for the same reason: probing is a
    /// side-effecting terminal query, not something `ui::draw` should do.
    /// `None` if the probe fails (piped stdio, a terminal that never answers
    /// the query) — avatars just don't render rather than the TUI refusing
    /// to start over what's ultimately a cosmetic feature.
    pub picker: Option<Picker>,
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
    /// The currently selected VTuber's avatar, already baked into a
    /// fixed-`AVATAR_COLS`x`AVATAR_ROWS` `Protocol` by `Picker::new_protocol`
    /// so `ui::draw_detail` can render it every frame with zero re-encoding
    /// work — matches how `ratatui-image` expects a static (non-resizing)
    /// slot to be used. `None` before it loads, on fetch/decode failure, or
    /// once `Picker::from_query_stdio` itself failed at startup (no
    /// supported protocol at all) — the avatar slot is cosmetic, so all
    /// three cases just mean "don't render one," never an error shown to
    /// the user.
    pub avatar: Option<Protocol>,
    /// Separate from `pending_detail_id` on purpose: the detail and avatar
    /// fetches are two independent tasks dispatched together by the same
    /// `Enter` press, but not guaranteed to resolve in the same order.
    /// `accept_detail` clearing `pending_detail_id` on arrival would silently
    /// drop every avatar that happens to resolve afterward, not just
    /// genuinely stale ones.
    pending_avatar_id: Option<String>,
    /// The focused stream's thumbnail, mirroring `avatar` above but keyed
    /// to *which stream/clip has focus* rather than which VTuber is
    /// selected. `None` whenever focus is on Clips (the backend has no
    /// `thumbnailUrl` for clips at all, see `routes::ClipInfo`) or the
    /// focused stream has none.
    pub thumbnail: Option<Protocol>,
    /// Keyed by thumbnail URL rather than a stream id — nothing upstream of
    /// this needs a dedicated stream identity, and the URL is already the
    /// natural unique key for "have we already decoded this image." Cleared
    /// in `begin_detail`, so it's naturally bounded to one VTuber's worth of
    /// streams rather than growing across the whole session.
    thumbnail_cache: HashMap<String, Protocol>,
    /// Same role as `pending_avatar_id`, but for `thumbnail` — also keyed by
    /// URL, since focus can move (and a fetch can be re-triggered) faster
    /// than a slow request resolves.
    pending_thumbnail_id: Option<String>,
    pub detail_focus: DetailFocus,
    pub stream_state: ListState,
    pub clip_state: ListState,
    /// Ids of currently-live VTubers, from the last `fetch_live_vtubers`.
    /// Empty until that fetch resolves — badges just don't show yet, same as
    /// the rest of the list before its own fetch resolves.
    pub live_ids: HashSet<String>,
    /// Ids that `watch::apply` flagged as `WentLive`/`BurstWentLive` on the
    /// *most recent* successful poll only — replaced wholesale each poll,
    /// not accumulated, so a highlight lasts exactly one `watch_interval_secs`
    /// before fading back to the plain `LIVE` badge.
    pub newly_live: HashSet<String>,
    /// Feeds `watch::apply` (Phase 7's ticker) the same fold `oshihub watch`
    /// itself uses, so "just went live" vs. "already live" comes from the
    /// same tested edge-detection rather than a reimplementation. Private:
    /// nothing outside `set_live` needs to touch the fold state directly.
    watch_state: WatchState,
    /// `L`'s toggle. When set, `visible_ids` narrows to `live_ids` members —
    /// nothing else needs to know about the filter, since selection and
    /// rendering both already go through `visible_ids`.
    pub live_only: bool,
    /// `/`'s typed text, matched case-insensitively against `VtuberRow.name`
    /// — client-side, not a request per keystroke, since the TUI already
    /// holds the full (small) list. Persists after `Enter` commits, so
    /// pressing `/` again continues editing rather than starting over.
    pub filter: String,
    /// Whether `/`'s text-input mode is active. While `true`, `event.rs`
    /// short-circuits before the normal keymap — every printable character
    /// is filter text, not a command, which is why `q`/`L`/etc. all need to
    /// keep working normally as soon as this goes back to `false`.
    pub filter_editing: bool,
    /// `a`'s in-progress URL text, and the parse error to show inline if
    /// `parse_channel_url` rejects it — validated *before* anything is sent,
    /// per the same rule `create_vtuber_channel` already follows.
    pub create_input: String,
    pub create_error: Option<String>,
    /// `e`'s in-progress form. `None` when the edit modal isn't open.
    pub edit: Option<EditForm>,
    /// Outcome of the last background action — `o`'s open-in-browser, or
    /// Phase 5's `s`/`d`/`a` — shown in the status bar until the next action
    /// or screen change clears it. One field rather than a separate
    /// success/failure pair: with two `Option`s it's possible for both to be
    /// `Some` at once and something has to arbitrate which wins, whereas one
    /// `Option<Result<..>>` makes that structurally impossible. Unlike a
    /// list/detail load failure, an action outcome has no screen of its own
    /// to show it on, so it has to surface here.
    pub status: Option<Result<String, String>>,
    /// `g`'s dashboard data for the VTuber it was opened on. `None` until
    /// the fetch resolves; `frequency_load` tracks that fetch the same way
    /// `detail_load` tracks Detail's.
    pub frequency: Option<FrequencyView>,
    pub frequency_load: LoadState,
    /// Same stale-response guard as `pending_detail_id`, for the frequency
    /// fetch: close the dashboard and open a different VTuber's before the
    /// first fetch resolves, and the late response no longer matches.
    pending_frequency_id: Option<String>,
    /// Name snapshot for the dashboard's title. Snapshotted at open rather
    /// than re-derived from `selected()` (the confirm-delete modal's
    /// approach) because the live ticker's `ensure_selection_valid` *can*
    /// move the selection under an open dashboard — modal input being
    /// exclusive is what makes re-deriving safe there, and that argument
    /// doesn't hold here.
    pub dashboard_name: String,
    /// Where `Esc`/`h` lands from the dashboard — `List` or `Detail`,
    /// whichever it was opened from. Detail's state is untouched while the
    /// dashboard is up, so returning is just a screen switch.
    dashboard_return: Screen,
    /// How many `o`/`s`/`d`/`a` actions are currently in flight. A counter
    /// rather than a bool: dispatching a second action before the first
    /// resolves must not let that first completion turn the spinner off
    /// while the second is still running.
    pub pending: u32,
    /// Advanced once per spinner tick while `pending > 0` (`mod.rs`'s
    /// `run_loop`). Just a counter — `ui.rs` owns the actual glyphs, same
    /// separation of concerns as the platform-casing comment above.
    pub spinner_frame: u32,
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
            // Must run after `setup_terminal` has already entered raw mode
            // (it writes an escape query and reads stdio for the reply) —
            // true by construction, since `mod.rs::run` only calls
            // `App::new` after `setup_terminal()?` returns.
            picker: Picker::from_query_stdio().ok(),
            detail: None,
            detail_load: LoadState::Loading,
            pending_detail_id: None,
            avatar: None,
            pending_avatar_id: None,
            thumbnail: None,
            thumbnail_cache: HashMap::new(),
            pending_thumbnail_id: None,
            detail_focus: DetailFocus::Streams,
            stream_state: ListState::default(),
            clip_state: ListState::default(),
            live_ids: HashSet::new(),
            newly_live: HashSet::new(),
            watch_state: WatchState::Seeding,
            live_only: false,
            filter: String::new(),
            filter_editing: false,
            create_input: String::new(),
            create_error: None,
            edit: None,
            frequency: None,
            frequency_load: LoadState::Loading,
            pending_frequency_id: None,
            dashboard_name: String::new(),
            dashboard_return: Screen::List,
            status: None,
            pending: 0,
            spinner_frame: 0,
        }
    }

    pub fn begin_action(&mut self) {
        self.pending += 1;
    }

    pub fn end_action(&mut self, result: Result<String, String>) {
        self.pending = self.pending.saturating_sub(1);
        self.status = Some(result);
    }

    pub fn advance_spinner(&mut self) {
        self.spinner_frame = self.spinner_frame.wrapping_add(1);
    }

    pub fn toggle_help(&mut self) {
        self.screen = match self.screen {
            Screen::Help => Screen::List,
            _ => Screen::Help,
        };
    }

    /// Indices into `items` that should actually render/be navigable right
    /// now — narrowed by `live_only` and/or `filter`, composed here rather
    /// than each keeping a separate mechanism. `list_state`'s selected index
    /// is always a position *into this*, not into `items` directly.
    pub fn visible_ids(&self) -> Vec<usize> {
        self.items
            .iter()
            .enumerate()
            .filter(|(_, row)| !self.live_only || self.live_ids.contains(&row.id))
            .filter(|(_, row)| self.matches_filter(row))
            .map(|(i, _)| i)
            .collect()
    }

    fn matches_filter(&self, row: &VtuberRow) -> bool {
        if self.filter.is_empty() {
            return true;
        }
        row.name.to_lowercase().contains(&self.filter.to_lowercase())
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

    /// Called on every successful live poll — the first one at startup and
    /// every one Phase 7's ticker fires afterward. A failed poll never
    /// reaches here at all (`handle_message` only calls this on `Ok`), which
    /// is what gives "a failed poll changes nothing" for free without this
    /// function needing to know about errors.
    pub fn set_live(&mut self, entries: Vec<LiveEntry>) {
        // Keyed the same way `watch::apply` internally keys its fold, so the
        // `StreamKey`s that fold hands back for `WentLive`/`BurstWentLive`
        // can be turned back into the vtuber ids `ui.rs` actually renders.
        let vtuber_by_key: HashMap<watch::StreamKey, String> = entries
            .iter()
            .map(|e| (watch::stream_key(e), e.vtuber.id.clone()))
            .collect();

        self.live_ids = entries.iter().map(|e| e.vtuber.id.clone()).collect();

        let state = std::mem::replace(&mut self.watch_state, WatchState::Seeding);
        let (next_state, actions) = watch::apply(state, Ok(&entries));
        self.watch_state = next_state;

        // The very first poll's `Seeded` deliberately doesn't highlight —
        // everything in it was already live before the TUI opened, same
        // reason `oshihub watch` doesn't notify for it either.
        self.newly_live = actions
            .into_iter()
            .flat_map(|action| match action {
                watch::Action::WentLive(key) => vec![key],
                watch::Action::BurstWentLive(keys) => keys,
                watch::Action::Seeded(_) => Vec::new(),
            })
            .filter_map(|key| vtuber_by_key.get(&key).cloned())
            .collect();

        if self.live_only {
            self.ensure_selection_valid();
        }
    }

    pub fn start_filter_edit(&mut self) {
        self.filter_editing = true;
    }

    /// `Enter`: stop editing, keep the typed text and the filtered view.
    pub fn commit_filter(&mut self) {
        self.filter_editing = false;
    }

    /// `Esc`: stop editing *and* drop the text, restoring the full list —
    /// distinct from `commit_filter`, which keeps the filter applied.
    pub fn clear_filter(&mut self) {
        self.filter.clear();
        self.filter_editing = false;
        self.ensure_selection_valid();
    }

    pub fn filter_push(&mut self, c: char) {
        self.filter.push(c);
        self.ensure_selection_valid();
    }

    pub fn filter_backspace(&mut self) {
        self.filter.pop();
        self.ensure_selection_valid();
    }

    pub fn go_back_to_list(&mut self) {
        self.screen = Screen::List;
        self.status = None;
    }

    pub fn begin_detail(&mut self, id: String) {
        self.screen = Screen::Detail;
        self.detail = None;
        self.detail_load = LoadState::Loading;
        self.avatar = None;
        self.pending_avatar_id = Some(id.clone());
        self.thumbnail = None;
        self.thumbnail_cache.clear();
        self.pending_thumbnail_id = None;
        self.pending_detail_id = Some(id);
        self.detail_focus = DetailFocus::Streams;
        self.stream_state = ListState::default();
        self.clip_state = ListState::default();
        self.status = None;
    }

    /// Opens the dashboard for the selected VTuber, remembering the screen
    /// it was opened from so `close_dashboard` can return there.
    pub fn begin_dashboard(&mut self, id: String, name: String) {
        self.dashboard_return = self.screen;
        self.screen = Screen::Dashboard;
        self.frequency = None;
        self.frequency_load = LoadState::Loading;
        self.pending_frequency_id = Some(id);
        self.dashboard_name = name;
        self.status = None;
    }

    pub fn close_dashboard(&mut self) {
        self.screen = self.dashboard_return;
        self.status = None;
    }

    /// Applies a frequency fetch's result, unless the user has since closed
    /// this dashboard and opened a different VTuber's (see
    /// `pending_frequency_id` — same guard as `accept_detail`'s).
    pub fn accept_frequency(
        &mut self,
        id: &str,
        result: Result<crate::routes::StreamFrequency, crate::routes::ApiError>,
    ) {
        if self.pending_frequency_id.as_deref() != Some(id) {
            return;
        }
        self.pending_frequency_id = None;
        match result {
            Ok(freq) => {
                self.frequency = Some(freq.into());
                self.frequency_load = LoadState::Loaded;
            }
            Err(e) => self.frequency_load = LoadState::Failed(e.to_string()),
        }
    }

    pub fn open_confirm_delete(&mut self) {
        self.status = None;
        self.screen = Screen::Modal(ModalKind::ConfirmDelete);
    }

    pub fn open_create_url(&mut self) {
        self.status = None;
        self.create_input.clear();
        self.create_error = None;
        self.screen = Screen::Modal(ModalKind::CreateUrl);
    }

    /// Validates `create_input` via the same `parse_channel_url` the actual
    /// request will use, *before* anything is dispatched — a guaranteed-fail
    /// URL shouldn't cost a spawned task and a round trip to find out.
    /// `Some(url)` on success (and closes the modal); `None` leaves it open
    /// with `create_error` set to show inline.
    pub fn try_submit_create(&mut self) -> Option<String> {
        match crate::routes::parse_channel_url(&self.create_input) {
            Ok(_) => {
                self.create_error = None;
                self.go_back_to_list();
                self.begin_action();
                Some(std::mem::take(&mut self.create_input))
            }
            Err(e) => {
                self.create_error = Some(e.to_string());
                None
            }
        }
    }

    /// Prefills straight from the selected `VtuberRow` — already holds
    /// everything needed (Phase 6 is what `english_name`/`native_name`/
    /// `photo`/`is_tracked` were added for), so opening the form costs no
    /// extra fetch.
    pub fn open_edit_form(&mut self) {
        let Some(row) = self.selected() else { return };
        let form = EditForm {
            id: row.id.clone(),
            display_name: row.name.clone(),
            name: row.native_name.clone(),
            english_name: row.english_name.clone(),
            photo: row.photo.clone(),
            org: row.org.clone().unwrap_or_default(),
            suborg: row.suborg.clone().unwrap_or_default(),
            is_tracked: row.is_tracked,
            focus: EditField::Name,
            error: None,
        };
        self.edit = Some(form);
        self.status = None;
        self.screen = Screen::Modal(ModalKind::Edit);
    }

    pub fn edit_next_field(&mut self) {
        if let Some(form) = &mut self.edit {
            form.focus = form.focus.next();
        }
    }

    pub fn edit_previous_field(&mut self) {
        if let Some(form) = &mut self.edit {
            form.focus = form.focus.previous();
        }
    }

    /// Only acts when `IsTracked` has focus — text fields need `Space` to
    /// insert a literal space (org names like "Hololive EN" have one), so
    /// this must never intercept it universally.
    pub fn edit_toggle_tracked(&mut self) {
        if let Some(form) = &mut self.edit
            && form.focus == EditField::IsTracked
        {
            form.is_tracked = !form.is_tracked;
        }
    }

    pub fn edit_push(&mut self, c: char) {
        if let Some(form) = &mut self.edit {
            match form.focus {
                EditField::Name => form.name.push(c),
                EditField::EnglishName => form.english_name.push(c),
                EditField::Photo => form.photo.push(c),
                EditField::Org => form.org.push(c),
                EditField::Suborg => form.suborg.push(c),
                EditField::IsTracked => {}
            }
            form.error = None;
        }
    }

    pub fn edit_backspace(&mut self) {
        if let Some(form) = &mut self.edit {
            match form.focus {
                EditField::Name => {
                    form.name.pop();
                }
                EditField::EnglishName => {
                    form.english_name.pop();
                }
                EditField::Photo => {
                    form.photo.pop();
                }
                EditField::Org => {
                    form.org.pop();
                }
                EditField::Suborg => {
                    form.suborg.pop();
                }
                EditField::IsTracked => {}
            }
        }
    }

    /// Validates `photo` — the one field the backend actually rejects a bad
    /// value for — *before* dispatching anything, same rule as
    /// `try_submit_create`. `Some(payload)` on success (and closes the
    /// modal); `None` leaves it open with `error` set to show inline.
    pub fn try_submit_edit(&mut self) -> Option<EditPayload> {
        let form = self.edit.as_mut()?;
        if !form.photo.is_empty() && !crate::routes::is_valid_url(&form.photo) {
            form.error = Some("Photo must be a valid URL".to_string());
            return None;
        }

        let form = self.edit.take()?;
        self.go_back_to_list();
        self.begin_action();
        Some(EditPayload {
            id: form.id,
            display_name: form.display_name,
            name: form.name,
            english_name: form.english_name,
            photo: form.photo,
            org: form.org,
            suborg: form.suborg,
            is_tracked: form.is_tracked,
        })
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

    /// Applies a decoded avatar, unless it's for a VTuber the user has since
    /// navigated away from (see `pending_avatar_id`). `image` is already
    /// `None` on fetch/decode failure — `mod.rs`'s fetch task collapses any
    /// `reqwest`/`image` error into `None` before sending, since a missing
    /// avatar has no error state of its own to show, it just doesn't render.
    pub fn accept_avatar(&mut self, id: &str, image: Option<image::DynamicImage>) {
        if self.pending_avatar_id.as_deref() != Some(id) {
            return;
        }
        self.pending_avatar_id = None;
        self.avatar = image.and_then(|img| {
            let size = Size { width: AVATAR_COLS, height: AVATAR_ROWS };
            self.picker.as_ref()?.new_protocol(img, size, Resize::Fit(None)).ok()
        });
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

    /// The focused stream's thumbnail URL. Always `None` for Clips —
    /// `routes::ClipInfo` has no `thumbnailUrl` field at all, the backend
    /// doesn't send one for clips.
    fn focused_thumbnail_url(&self) -> Option<String> {
        let detail = self.detail.as_ref()?;
        match self.detail_focus {
            DetailFocus::Streams => {
                let i = self.stream_state.selected()?;
                detail.streams.get(i)?.thumbnail_url.clone()
            }
            DetailFocus::Clips => None,
        }
    }

    /// Call after any focus change (`detail_next`/`detail_previous`/
    /// `toggle_detail_focus`, and once from `accept_detail` for the initial
    /// selection). Updates `self.thumbnail` immediately — from the cache if
    /// this URL was already fetched, or to `None` while a fresh one is in
    /// flight, so the *previous* stream's thumbnail never lingers on screen
    /// under a new selection. Returns the URL to fetch, if a fetch is
    /// actually needed.
    pub fn sync_thumbnail_focus(&mut self) -> Option<String> {
        let url = self.focused_thumbnail_url();
        self.thumbnail = url.as_ref().and_then(|u| self.thumbnail_cache.get(u).cloned());
        match url {
            Some(url) if !self.thumbnail_cache.contains_key(&url) => {
                self.pending_thumbnail_id = Some(url.clone());
                Some(url)
            }
            _ => {
                self.pending_thumbnail_id = None;
                None
            }
        }
    }

    /// Applies a decoded thumbnail, unless focus has since moved to a
    /// different stream (see `pending_thumbnail_id`). Successfully decoded
    /// images are cached by URL so arrowing back over an already-seen
    /// stream is instant and doesn't re-fetch.
    pub fn accept_thumbnail(&mut self, url: &str, image: Option<image::DynamicImage>) {
        if self.pending_thumbnail_id.as_deref() != Some(url) {
            return;
        }
        self.pending_thumbnail_id = None;
        let Some(protocol) = image.and_then(|img| {
            let size = Size { width: THUMB_COLS, height: THUMB_ROWS };
            self.picker.as_ref()?.new_protocol(img, size, Resize::Fit(None)).ok()
        }) else {
            return;
        };
        self.thumbnail_cache.insert(url.to_string(), protocol.clone());
        self.thumbnail = Some(protocol);
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
