# TUI development — plan and running checklist

Status: **Phases 0–7 implemented — TUI v0.1 complete.** Phase 7.5 (polish /
thumbnails) and Phase 8 (dashboard) both deferred to a future release.
Branch `tui`.

`oshihub` has nine one-shot CLI subcommands. This document plans mapping all of
them onto a single `ratatui` interface, plus the one backend capability no CLI
command ever exposed (`PUT /api/vtubers/:id`). It is the roadmap *and* the
checklist — tick boxes as phases land.

The CLI is not going away. Every subcommand keeps working, a bare `oshihub`
keeps printing help, and `oshihub tui` stays an explicit subcommand rather than
a default. The TUI is a second front end over the same `routes.rs`, not a
replacement for the first.

## How this work gets reviewed

**The user reviews TUI features by running the binary in their own terminal.**
This is a deliberate departure from the rest of the repo, where "verify against
the real running system" usually means the assistant drives it. A TUI is the
one thing an assistant cannot meaningfully check: correctness here is *how it
looks and feels*, and a scripted keystroke replay confirms neither.

So:

1. **No unit tests for render or navigation code.** A test asserting that
   `next()` increments a selection index is noise. Tests are written only where
   logic is genuinely non-obvious *and* pure — the bar `watch.rs`'s rules meet
   and render code does not.
2. **No scripted pty runs to self-verify.** No `script`, no pty harness, no
   synthetic keystrokes.
3. **`cargo build` must still pass with no warnings.** A compile error is not
   an "obvious pass".
4. **Stop at the end of every phase**, state the exact command and the exact
   keys to press, and wait. Commit only after the user confirms.
5. **Do not start the next phase while waiting.**

## Command mapping

| CLI command | Alias | TUI equivalent | Phase |
|---|---|---|---|
| `list` | `l` | Main list pane | ✅ 0 |
| `config` | — | `?` help/status overlay | ✅ 1 |
| `lookup <name>` (detail half) | `lk` | `Enter` → detail view | ✅ 2 |
| `jump <name>` | `j` | `o` → open in browser | ✅ 2 |
| `live` | `lv` | Live badges + `L` live-only view | ✅ 3 |
| `lookup <name>` (search half) | `lk` | `/` → incremental filter | ✅ 4 |
| `sync <name>` | `s` | `s` on selection | ✅ 5 |
| `delete <name>` | `d` | `d` + confirm modal | ✅ 5 |
| `create <url>` | `c` | `a` → URL input modal | ✅ 5 |
| *(none — known gap)* | — | `e` → edit modal | ✅ 6 |
| `watch` | `w` | Auto-refresh in place | ✅ 7 |
| *(none — future)* | — | Dashboard / charts | 8 |

## Keymap

Vim-flavoured, because the existing list already uses `j`/`k`.

```
j / k / ↓ / ↑   move            Enter   detail view
/               filter          o       open channel in browser
L               live-only       s       sync selected
?               help overlay    d       delete selected (confirms)
r               refresh         a       add from URL
Esc / h         back            e       edit selected
q               quit
```

---

## Traps

Verified, not theoretical. Most cost real debugging time to find and none are
discoverable from the API surface.

**`Box<dyn Error>` is not `Send`, so it cannot cross `tokio::spawn`.** Most of
`routes.rs` returns it. A background fetch task cannot. `ApiError`
(`cli/src/routes.rs`) already exists, is `Send + Sync`, and `fetch_live_vtubers`
already uses it — extend that, don't invent a second error type. This is why
the error migration is Phase 1a and blocks everything after it.

**`crossterm`'s `event-stream` feature is not enabled.** `cli/Cargo.toml` has a
bare `crossterm = "0.29"`. `EventStream` — the async input source that replaces
the blocking `event::poll` — needs `features = ["event-stream"]`.

**`theme.rs` cannot be reused by the TUI.** It returns `ColoredString` from the
`colored` crate, which embeds raw ANSI escape bytes. Written into a ratatui
buffer those render as literal text, not colour. The TUI needs a parallel
palette returning `ratatui::style::Style`. The two cannot be merged — they
target different rendering models — so the convention becomes: *the same
concept keeps the same colour in both files*, and both get updated together.

**`viuer` and ratatui fight over the screen.** `viuer` writes graphics escapes
directly at the cursor position; ratatui repaints every cell each frame with no
idea those cells are occupied. `routes::print_thumbnail` and
`print_stream_thumbnail` therefore **cannot be called from inside the TUI**.
Images need either the `ratatui-image` crate (a real widget that participates
in the buffer) or halfblock characters decoded via the already-present `image`
crate. See Phase 2.

**`run_loop` is sync and blocks the tokio runtime.** Fine while the only fetch
happens before the loop starts. The moment anything fetches *during* the loop,
it must become `tokio::select!` — otherwise the spawned task cannot progress
while input is being polled.

**A failed poll must change nothing.** `watch.rs`'s two rules (a failed poll is
not "nobody is live"; eviction takes two consecutive misses) mirror the
backend's `scheduler.ts` guards. Phase 7 reuses `watch::apply` verbatim rather
than reimplementing them.

**MongoDB is shared production Atlas.** Local dev writes to the same data the
deployed backend serves. Phase 5 onward mutates real records — there is no
sandbox, and `d` deletes cascade to streams, clips, and snapshots.

---

## Backend endpoints the TUI uses

All under `/api/*`, all requiring `Authorization: Bearer <token>` when the
backend has `API_TOKEN` set. `config::client()` attaches it already.

```
GET    /api/vtubers                   → [VtuberChannel]
GET    /api/vtubers?name=<partial>    → [VtuberChannel]     server-side search
GET    /api/vtubers/live              → [{ vtuber, stream }]
GET    /api/vtubers/:id               → { vtuber, streams[], clips[], snapshots[] }
GET    /api/vtubers/:id/profile-url   → { url }
POST   /api/vtubers                   ← { platform, channelId }
PUT    /api/vtubers/:id               ← { name?, englishName?, photo?,
                                          isTracked?, org?, suborg? }
                                      → { message, vtuber }
DELETE /api/vtubers/:id
POST   /api/sync/{holodex|youtube|twitch}?id=<id>&force=true
POST   /api/sync/all
```

Shape notes worth knowing before modelling anything:

- `englishName` can be an **empty string** rather than absent for some Twitch
  channels — fall back to `name`, don't assume presence means non-empty.
- `org`/`suborg` are **absent entirely** for Twitch-sourced records, not null.
- `streams[]`: `title`, `url`, `externalId`, `thumbnailUrl` (optional, nullable,
  *or* empty-string), `startTime`, `endTime` (nullable), `duration` (nullable),
  `status` ∈ `upcoming|live|ended|unknown`, `platform`, `sourceApi`.
  `StreamInfo` models a subset — extend it, don't add a parallel struct. Serde
  ignores undeclared fields, so partial models are safe.
- `clips[]`: `title`, `url`, `viewCount`, `externalId`, `createdAt`,
  `sourceStreamId` (nullable).
- `snapshots[]`: `subscriberCount`, `viewCount`, `capturedAt`, `sourceApi`.
  Append-only, newest-first, capped at 10 by the endpoint. Not modelled in the
  CLI yet — only Phase 8 needs it.
- `PUT` validates `photo` as a **URL**; the other fields are free strings.

---

# Phase 0 — List view

**Done** (`32ee84f`). Read-only list of tracked VTubers, `j`/`k` navigation,
`q`/`Esc` to quit.

- [x] `tui/` module: `mod.rs` (loop + terminal lifecycle), `app.rs` (state),
      `event.rs` (input), `ui.rs` (render)
- [x] `Tui` clap subcommand, alias `t`
- [x] `VtuberChannel` → `VtuberRow` mapper at the boundary
- [x] Panic hook restoring the terminal before the default hook prints

---

# Phase 1 — Foundation

**Done**, reviewed by running. No new features of its own — everything after
it depends on all three parts.

### 1a. Migrate `routes.rs` to `ApiError`

- [x] `fetch_vtubers`, `fetch_vtuber_detail`, `fetch_profile_url`,
      `lookup_by_name`, `create_vtuber_channel`, `delete_vtuber_channel`,
      `sync_vtuber_channels` return `Result<_, ApiError>`. Also added
      `ApiError::Invalid(String)` for local validation failures (a bad URL, a
      name matching nothing) that never reach the network, so
      `parse_channel_url` and the "no VTuber found" paths stay on the same
      error type instead of a second one.
- [x] Split the `println!` out of the three mutating functions — they return
      an outcome (`delete_vtuber_channel` now returns the deleted
      `VtuberChannel`) and `main.rs` prints it
- [x] Drop the `map_err(|e| anyhow!("{e}"))` bridge in `tui/mod.rs`

`main.rs` keeps compiling unchanged: `?` still converts `ApiError` into
`Box<dyn Error>` via the blanket `From` impl. Side benefit — the TUI can now
distinguish `Unauthorized` ("check your token") from `Transport` ("backend
unreachable"), which is also the `Todo.md` item about raw `reqwest::Error`
Debug dumps on an unreachable backend.

### 1b. Async event loop

- [x] Enable `features = ["event-stream"]` on crossterm (plus a new
      `futures-util` dependency for `StreamExt`, needed to call `.next()` on
      `EventStream`)
- [x] Replace blocking `event::poll` with `tokio::select!` over
      `EventStream` and an `mpsc::Receiver<Message>`
- [x] `run_loop` becomes `async`

The initial vtuber fetch also moved from a blocking pre-loop `await` into a
task spawned onto the channel — not just plumbing-for-its-own-sake: with the
blocking version, `LoadState::Loading` was set but the fetch always resolved
before the first frame drew, so it could never actually render. Spawning it
means "Loading tracked VTubers..." now has a real chance to show.

`Message` only has the one variant this phase needs:

```rust
enum Message {
    Vtubers(Result<Vec<VtuberRow>, ApiError>),
}
```

`Detail`, `ActionDone`, and `Tick` are deferred to Phases 2, 5, and 7 —
this is a *binary* crate, so `cargo build`'s dead-code lint fires on any
unreachable item regardless of `pub`, unlike in a library crate. Add each
variant in the phase that actually constructs it.

`ui::draw` stays a pure fn of `&App`. The immediate-mode model does not change
— only the input source does.

### 1c. `tui/theme.rs`

- [x] Ratatui `Style` twins of the two concepts this phase renders: `name`
      bright cyan bold, `muted` dark gray (using an explicit `Color`, not a
      `DIM` modifier — terminal support for `DIM` is inconsistent, and
      `DarkGray` is what Phase 0 already shipped and verified). `live`,
      `url`, and `status_tag` are deferred to Phases 3 and 2, when a live
      badge and a URL first appear on screen — same dead-code constraint as
      `Message` above.
- [x] Move `ui.rs`'s inline `Color::DarkGray` into it

### Also

- [x] `Screen` enum on `App` — `List` / `Help` only for now; `Detail` and
      `Modal(..)` are added in Phases 2 and 5 when something actually
      constructs them
- [x] Status bar: key hints, context-sensitive to `Screen`. A "last error"
      slot is deferred — Phase 1 has no interaction that can fail without
      already replacing the whole view via `LoadState::Failed`; it becomes
      meaningful once Phase 2/5 can fail without nuking the list
- [x] `?` help overlay, doubling as `oshihub config` — resolved backend URL
      and *whether* a token is set and from where. **Never the token
      itself**, matching the existing rule in `Commands::Config`. Resolved
      once into `App` fields at startup (`config_url`/`config_source`/
      `config_token`) rather than read from `ui::draw`, so `draw` stays a
      pure function of `&App` alone.

**Review:** done — list renders and navigates; `?` opens and closes and shows
the right backend URL; status bar visible; `q` still quits cleanly.

---

# Phase 2 — Detail view and open in browser

Maps `lookup`'s detail half and `jump`. **Done**, reviewed by running.

- [x] `Enter` → Detail screen via `fetch_vtuber_detail`, dispatched as a
      background task so the UI stays responsive
- [x] Shows name, org/suborg, platform, live status, recent streams (status tag
      + title + URL), recent clips (title + view count)
- [x] `o` → `fetch_profile_url` + `open::that` on the list screen. Contextual
      inside Detail (see below) rather than always meaning "open the
      channel" — a deliberate choice, not the originally planned behavior
- [x] `Esc` / `h` → back

`Enter`'s background fetch guards against a stale response: if the user backs
out and opens a *different* VTuber before the first fetch resolves, the late
response's id no longer matches `App::pending_detail_id` and is dropped
rather than overwriting the wrong detail screen. `o`'s failures (bad token, no
browser handler) have no screen of their own to show on, so they surface in a
new `last_error` status-bar field, cleared on the next action or screen
change.

**Thumbnails are deliberately excluded from this phase.** `viuer` cannot be
used (see Traps). The options are `ratatui-image`, or halfblocks via the
`image` crate already in the tree. Images are the single most likely thing to
make the whole view look broken, so they get their own phase later rather than
blocking the rest of the mapping. `cli/IMAGE_RENDERING.md` has background.

### Open URL per stream/clip

**Done**, reviewed by running. Turned out mechanically lighter than the
channel-`o` path above, as expected — `s.url`/`c.url` are already sitting in
`app.detail` by the time Detail renders, so opening one needs no fetch, just
`open::that` directly (`Command::OpenUrl`, alongside the existing
`Command::OpenProfile` which still resolves a URL first).

- [x] Streams and clips are real `List` widgets (`App::stream_state`/
      `clip_state`), not static lines. `Tab` switches focus between the two
      panes; the focused one gets a bright border, reversed highlight, and
      `>` marker, the unfocused one stays dim
- [x] `o` is contextual inside Detail: opens whichever stream/clip currently
      has focus, rather than the channel. To open the channel from Detail,
      back out to List (`Esc`/`h`) and press `o` there — an explicit tradeoff
      picked over adding a second "open" key, see `App::focused_url`
- [x] `j`/`k`/arrows move within whichever pane has focus, sharing the same
      wrap-around `cycle()` helper the main list's `next`/`previous` now also
      use (four near-identical call sites collapsed into one function)

**Review:** `Enter` shows correct streams/clips; `o` opens the right channel
from List; `Tab` moves focus between streams/clips with a visible highlight
change; `j`/`k` move and wrap within the focused pane; `o` inside Detail opens
the focused stream/clip's URL; an empty pane's `Tab`/`j`/`k`/`o` all no-op
instead of erroring; `Esc` returns; display stays clean.

---

# Phase 3 — Live

Maps `live`. **Done**, reviewed by running.

- [x] Live badge on list rows, using `tui/theme.rs`'s green (`live_status`,
      already added in Phase 2 for Detail's status line and now reused here
      rather than adding a second function for the same colour)
- [x] `L` toggles a live-only view
- [x] Reuse `fetch_live_vtubers` and `watch::dedupe_one_per_vtuber` rather than
      reimplementing dedup — the spawned fetch pipes straight through
      `dedupe_one_per_vtuber` before anything reaches `App`

`fetch_live_vtubers` is dispatched as its own background task at startup,
alongside the vtubers fetch, into a new `Message::Live` variant. Selection
handling turned out to be the real substance of this phase: rather than
filtering `app.items` into a second copy when `L` is on, `App::visible_ids()`
is now the single source of truth for "which indices are navigable/rendered
right now," and `list_state`'s selected index is always a position *into
that*, not into `items` directly. `next`/`previous`/`selected`/`draw_list`
all go through it, so toggling the filter can't desync what's stored from
what's shown. `ensure_selection_valid` snaps the selection back into range
when the visible set shrinks out from under it (toggling `L`, or the live set
itself updating after the fact) — not strictly required for safety (an
out-of-range index just renders nothing highlighted, `cycle()` self-heals it
on the next keypress), but avoids a needless dead-cursor moment.

This indirection is deliberately reusable: Phase 4's incremental search
filter needs the same shape (a shrinking visible set, a selection that has to
stay valid across it), so `/` should widen `visible_ids`'s predicate rather
than invent a parallel mechanism.

**Review:** badges match `oshihub live`; `L` filters and toggles back;
selection stays valid (never on a hidden row) across both toggles; an empty
live set shows "No one is live right now." instead of an empty list; `j`/`k`/
`Enter`/`o` all operate on the visible (filtered) set while `L` is active.

---

# Phase 4 — Search and filter

Maps `lookup`'s search half. **Done**, reviewed by running.

- [x] `/` enters filter mode, filtering incrementally as you type
- [x] `Esc` clears, `Enter` commits and returns focus to the list
- [x] Selection stays valid as the list shrinks

**Client-side filtering, not a request per keystroke.** `oshihub lookup` hits
`GET /api/vtubers?name=` because a one-shot command has nothing cached. The TUI
already holds the full list and the dataset is tiny, so filtering locally is
both faster and kinder to the backend.

Turned out to be exactly the reuse Phase 3 set up for: `App::visible_ids()`
now chains a `live_only` predicate and a `filter` predicate, so `L` and `/`
compose freely instead of needing to know about each other — no new
selection-safety mechanism, `ensure_selection_valid` from Phase 3 covers this
too.

`filter_editing` is a short-circuit at the *top* of `handle_event`, not a
guard threaded through the existing match arms — while typing a query,
`q`/`L`/`j`/`k`/etc. are all literal characters, not commands, and getting
that right per-arm would be one missed guard away from `q` quitting the app
mid-search. `Esc` and `Enter` diverge deliberately: `Esc` clears the text
*and* exits editing (`App::clear_filter`); `Enter` only exits editing,
keeping the filter applied (`App::commit_filter`) — pressing `/` again
resumes editing the same text rather than starting over. Arrow keys (not
`j`/`k`) still move the selection live while editing, matching how most
fuzzy-finders behave with their input focused.

**Review:** typing filters live; selection never points off the end; `Esc`
restores the full list; `Enter` keeps the filter applied and returns to
normal navigation; re-opening `/` continues editing rather than clearing;
`L` and `/` compose; no matches shows "No VTubers match '...'" instead of an
empty list.

---

# Phase 5 — Actions: sync, delete, create

Maps `sync`, `delete`, `create`. **First phase that mutates the shared
production database.** **Done**, reviewed by running.

- [x] `s` → sync selected, as a background task with status feedback. Picks
      the sync path from `source` via a new `routes::sync_vtuber_channel_by_id`
      (and `delete_vtuber_channel_by_id`) — the TUI already has the exact
      record selected, so both mutate by id directly rather than going
      through the CLI's name-based lookup, which the name-based functions
      now call into instead of duplicating
- [x] `d` → **confirm modal naming the VTuber**, then delete
- [x] `a` → URL input modal; `parse_channel_url` is now `pub(crate)` and
      reused so validation matches the CLI exactly, with parse errors shown
      inline *before* anything is dispatched — a guaranteed-fail URL never
      spawns a task
- [x] All three refresh the list on success (`Message::ActionDone`; a
      successful result signals `run_loop` to re-`spawn_fetch_vtubers`)

The delete confirmation is mandatory, not polish. A stray keypress in a TUI is
far easier than mistyping `oshihub delete <name>`, and the backend cascades the
delete to streams, clips, and snapshots.

`Screen::Modal(ModalKind)` finally gets its real construction sites
(`ConfirmDelete`, `CreateUrl`). Modal input is exclusive — `event.rs`
short-circuits on `Screen::Modal(_)` before the normal keymap, same
reasoning as `filter_editing`, so a stray `q` can't slip through and quit
under an open modal. The confirm-delete modal re-derives the VTuber from
`app.selected()` at confirm time rather than snapshotting when it opened;
since input is exclusive, selection can't drift while it's open, so the
prompt text and the delete it fires are guaranteed to name the same VTuber
by construction.

**Post-review adjustments**, both from user feedback after the initial pass:
- Action outcomes used to take over the *entire* bottom row, hiding the key
  hints exactly when you'd want them. Split into two permanent rows: an
  activity line (status/spinner) and a hints line that's now always visible.
- No visual cue existed for `o`/`s`/`d`/`a` while in flight. Added
  `App.pending: u32` (a counter, not a bool, so a second action dispatched
  before the first resolves can't let that first completion hide the
  spinner too early) driving a `|/-\` glyph, advanced by a `tokio::select!`
  branch gated on `if app.pending > 0` — never even polled while nothing's
  running, so it's free at rest.

**Known issue at the time, since fixed:** live badges didn't update after
startup, so a VTuber created already-live wouldn't show `LIVE` until the TUI
restarted, even after a manual `s` corrected the backend's own data. Phase
7's auto-refresh ticker closed this — see that phase's notes.

**Review:** sync a VTuber; create one from a real URL; delete it again and
confirm the modal blocks an accidental `d`; spinner and hints both visible
throughout each action.

---

# Phase 6 — Edit

Closed the "No `update` CLI command" entry under CLAUDE.md's Known gaps.
**Done**, reviewed by running.

- [x] `e` → prefilled form modal over `name`, `englishName`, `photo`,
      `isTracked`, `org`, `suborg` (all optional; `photo` must be a valid URL)
- [x] New `routes::update_vtuber_channel`
- [x] Removed the Known-gaps line from `CLAUDE.md`

`isTracked` as a toggle is the most useful field here — it is what the list
filters on. Toggling it off and saving relies on the same refresh-on-success
plumbing Phase 5 built (re-fetching the tracked list, already filtered on
`isTracked` both server- and client-side), so "leaves the list" needed no
special-case code.

Prefilling costs no extra fetch: the list's own load already returns full
`VtuberChannel`s, so `VtuberRow` just needed to stop discarding
`englishName`/native `name`/`photo`/`isTracked` on the way to the display
view (it only kept the already-collapsed display `name` before). Photo is
validated as a URL *before* dispatching anything, same rule as `a`'s create
flow — via `reqwest::Url::parse` (a re-export already available transitively
through `reqwest`), not a new dependency for one check.

The one genuinely tricky key: `Space` means two different things depending
on which field has focus — toggle `isTracked` there, or insert a literal
space everywhere else (org names like "Hololive EN" routinely have one).
`edit_toggle_tracked` only acts when `IsTracked` is focused, so `Space`
falls through to normal text input everywhere else without `event.rs`
needing to special-case anything.

**Review:** rename a VTuber and see it persist; toggle `isTracked` off and
watch it leave the list; an invalid photo URL shows inline and blocks
submission; `Tab`/`Shift+Tab`/`↑`/`↓` all cycle fields; typing a space into
org/suborg works normally.

---

# Phase 7 — Auto-refresh

Maps `watch`. **Done**, reviewed by running. This closes out TUI v0.1 — every
row in Command mapping above is now ✅ except the deferred Phase 8 dashboard.

- [x] Background ticker on the configured `watch_interval_secs`, feeding the
      Phase 1 channel
- [x] Reuse `watch::apply(state, poll) -> (state, actions)` **verbatim**
- [x] Rows update in place; newly-live VTubers highlight

`apply` is pure, has 17 existing tests, and encodes both load-bearing rules
from Traps — nothing here reimplements them. `tui/mod.rs`'s old one-shot
`spawn_fetch_live` (fired once at `run()`'s startup, never again) became
`spawn_live_ticker`: an immediate poll followed by a self-rescheduling
`sleep(watch_interval_secs)` loop, for the life of the session. Each
successful poll is handed to `App::set_live`, which now folds it through a
private `watch_state: WatchState` via `watch::apply` instead of just
overwriting `live_ids` — the same edge-detection `oshihub watch` itself
runs, reused rather than re-derived. `newly_live: HashSet<String>` holds only
the ids from *that poll's* `WentLive`/`BurstWentLive` actions (never
`Seeded` — matching why `watch` doesn't notify for it either), replaced
wholesale each poll rather than accumulated, so a highlight lasts exactly one
interval before the row settles back to the plain green `LIVE` badge
(`theme::just_went_live`, reversed video to stand out from `live_status`).

This also closes the live-badge staleness gap logged during Phase 5's
review: creating a VTuber already live on Twitch no longer needs a TUI
restart (or a manual `s` sync) to show `LIVE` — the next tick of the ticker
picks it up on its own.

**Polling, not SSE.** `CLAUDE.md` and `Todo.md` record this reversal: polling
won because this runs on a laptop, and a pushed event fired while the lid is
shut is gone, whereas the next poll returns current truth.

**The TUI does not send desktop notifications.** `watch` takes no lock, so a
terminal instance plus the enabled `oshihub-watch.service` user unit already
double-notify; a notifying TUI would be a third. In-place visual updates only.

**Review:** `OSHIHUB_WATCH_INTERVAL=15 cargo run -- tui`, left open across a
real go-live (or an `a`-create of an already-live channel) — the row picked
up `LIVE` with no keypress, reversed for one poll, then settled to the plain
badge. VTubers already live at startup never got the highlight.

---

# Phase 7.5 — Polish & thumbnails (in progress)

A holding pen for this doc rather than an open-ended Known gaps list growing
without a home. Two things belong here before Phase 8:

- [ ] **Thumbnails inside the TUI.** `lookup`/`live` render them via `viuer`,
      which writes graphics escapes straight at wherever the cursor sits.
      Ratatui repaints every cell from a fresh buffer each frame and has no
      idea those cells hold an image — the two can't currently coexist (see
      Traps). Needs an actual design pass: either a redraw-aware image crate
      (`ratatui-image` is the obvious candidate to evaluate first) or a
      half-block/ANSI-art approach drawn as ordinary styled cells so ratatui
      owns every pixel it's responsible for. Not investigated yet — this
      entry is the placeholder to come back to, not a design decision.
- [x] **General polish pass.** Catching small UX rough edges as they turn up
      rather than opening a one-off phase per fix. Landed so far, all
      reviewed by running:
    - [x] `o` (open browser, List and Detail) no longer calls
          `App::begin_action`. It used to, but a successful `open::that`
          sends no message back to end it — only the failure path did — so
          `app.pending` never decremented and the spinner spun forever after
          the first successful open. `o`'s failure path now sets
          `app.status` directly instead of routing through `end_action`,
          so it can no longer steal a decrement from an unrelated `s`/`d`/`a`
          action still in flight either.
    - [x] Hints bar truncates with a trailing `…` on a narrow terminal
          instead of letting `Paragraph`'s own (silent, mid-word) truncation
          cut a key binding off with no sign anything's missing.
    - [x] Confirm-delete modal no longer clips its message. It used
          `centered_rect`'s `Percentage` height, which a normal ~24-row
          terminal can undershoot for the 5 lines of content, and no `Wrap`,
          so the fixed message (52 chars) didn't fit a 50%-wide popup's
          inner width on a standard 80-column terminal and lost its tail.
          Now uses a fixed-row-count popup (`centered_rect_fixed_height`)
          plus `Wrap { trim: true }`.
    - [x] Version number (`CARGO_PKG_VERSION`, compile-time) added to the
          far right of the hints bar, in its own reserved layout slot so
          truncation on a narrow terminal eats into the hint text before it
          ever touches the version. Styled `Style::default()` to match the
          unstyled border colour used everywhere else in the TUI, not
          `theme::muted()`'s dim hint-text colour.

Deliberately light on detail for thumbnails — the point right now is
reserving the slot before Phase 8, not committing to a design.

---

# Phase 8 — Dashboard (deferred)

From `Todo.md`'s dashboard section. All aggregations over data the schema
already collects — no new tracking, just querying what's there.

- [ ] Stream frequency — bucket `Stream.startTime`
- [ ] Follower/subscriber trend — `StatSnapshot` is append-only, so the line
      data exists as soon as two snapshots do. Show the delta ("+500 this
      week") alongside the raw count; it reads better at a glance
- [ ] Average stream duration trend — `Stream.duration` is already computed
- [ ] Model `snapshots[]` in the CLI

Before designing any aggregation endpoint, check what shape ratatui's
`Sparkline`/`Chart` widgets actually want data in — the terminal-charting half
is the harder one, not the data. Out of scope for v0.1; first thing on top of
it once picked back up.

---

## Known gaps

- Thumbnails are absent from the TUI entirely, pending the `ratatui-image` vs
  halfblocks decision (Phase 2).
- The TUI filters on `isTracked` where `oshihub list` does not. The counts
  happen to match today because everything in the database is tracked.
- No lock, same as `watch` — nothing stops two TUI instances, or a TUI
  session and the enabled `oshihub-watch.service` user unit, polling
  `/api/vtubers/live` independently. Harmless (each just re-derives the same
  state), but redundant load on the backend.
