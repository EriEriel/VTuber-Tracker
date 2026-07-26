# TUI development — plan and running checklist

Status: **Phases 0–2 implemented**, Phases 3–7 planned, Phase 8 deferred.
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
| `config` | — | `?` help/status overlay | 1 |
| `lookup <name>` (detail half) | `lk` | `Enter` → detail view | 2 |
| `jump <name>` | `j` | `o` → open in browser | 2 |
| `live` | `lv` | Live badges + `L` live-only view | 3 |
| `lookup <name>` (search half) | `lk` | `/` → incremental filter | 4 |
| `sync <name>` | `s` | `s` on selection | 5 |
| `delete <name>` | `d` | `d` + confirm modal | 5 |
| `create <url>` | `c` | `a` → URL input modal | 5 |
| *(none — known gap)* | — | `e` → edit modal | 6 |
| `watch` | `w` | Auto-refresh in place | 7 |
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

Maps `live`.

- [ ] Live badge on list rows, using `tui/theme.rs`'s green
- [ ] `L` toggles a live-only view
- [ ] Reuse `fetch_live_vtubers` and `watch::dedupe_one_per_vtuber` rather than
      reimplementing dedup

**Review:** badges match `oshihub live`; `L` filters and toggles back.

---

# Phase 4 — Search and filter

Maps `lookup`'s search half.

- [ ] `/` enters filter mode, filtering incrementally as you type
- [ ] `Esc` clears, `Enter` commits and returns focus to the list
- [ ] Selection stays valid as the list shrinks

**Client-side filtering, not a request per keystroke.** `oshihub lookup` hits
`GET /api/vtubers?name=` because a one-shot command has nothing cached. The TUI
already holds the full list and the dataset is tiny, so filtering locally is
both faster and kinder to the backend.

**Review:** typing filters live; selection never points off the end; `Esc`
restores the full list.

---

# Phase 5 — Actions: sync, delete, create

Maps `sync`, `delete`, `create`. **First phase that mutates the shared
production database.**

- [ ] `s` → sync selected, as a background task with status feedback. Pick the
      sync path from `source` exactly as `main.rs` does: `holodex` → `holodex`,
      `youtube_api` → `youtube`, `twitch_api` → `twitch`
- [ ] `d` → **confirm modal naming the VTuber**, then delete
- [ ] `a` → URL input modal; make `parse_channel_url` `pub(crate)` and reuse it
      so validation matches the CLI exactly, showing parse errors inline before
      sending
- [ ] All three refresh the list on success

The delete confirmation is mandatory, not polish. A stray keypress in a TUI is
far easier than mistyping `oshihub delete <name>`, and the backend cascades the
delete to streams, clips, and snapshots.

**Review:** sync a VTuber; create one from a real URL; delete it again and
confirm the modal blocks an accidental `d`.

---

# Phase 6 — Edit

Closes the "No `update` CLI command" entry under CLAUDE.md's Known gaps.

- [ ] `e` → prefilled form modal over `name`, `englishName`, `photo`,
      `isTracked`, `org`, `suborg` (all optional; `photo` must be a valid URL)
- [ ] New `routes::update_vtuber_channel`
- [ ] Remove the Known-gaps line from `CLAUDE.md` once this lands

`isTracked` as a toggle is the most useful field here — it is what the list
filters on.

**Review:** rename a VTuber and see it persist; toggle `isTracked` off and
watch it leave the list.

---

# Phase 7 — Auto-refresh

Maps `watch`.

- [ ] Background ticker on the configured `watch_interval_secs`, feeding the
      Phase 1 channel
- [ ] Reuse `watch::apply(state, poll) -> (state, actions)` **verbatim**
- [ ] Rows update in place; newly-live VTubers highlight

`apply` is pure, has 17 existing tests, and encodes both load-bearing rules
from Traps. Do not reimplement either.

**Polling, not SSE.** `CLAUDE.md` and `Todo.md` record this reversal: polling
won because this runs on a laptop, and a pushed event fired while the lid is
shut is gone, whereas the next poll returns current truth.

**The TUI does not send desktop notifications.** `watch` takes no lock, so a
terminal instance plus the enabled `oshihub-watch.service` user unit already
double-notify; a notifying TUI would be a third. In-place visual updates only.

**Review:** leave the TUI open across a real go-live; the row updates within
one interval with no manual refresh.

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
is the harder one, not the data. Out of scope until Phase 7 lands.

---

## Known gaps

- Thumbnails are absent from the TUI entirely, pending the `ratatui-image` vs
  halfblocks decision (Phase 2).
- The TUI filters on `isTracked` where `oshihub list` does not. The counts
  happen to match today because everything in the database is tracked.
- No lock, same as `watch` — nothing stops two TUI instances polling
  independently once Phase 7 lands.
