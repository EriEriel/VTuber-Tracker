# CLAUDE.md

Guidance for Claude Code (claude.ai/code) working in this repository.

A polyglot repo: a Hono/Bun REST API aggregating VTuber data from HoloDex,
YouTube, and Twitch into MongoDB, plus `oshihub`, a Rust CLI client.

```
backend/   Hono/Bun REST API (TypeScript)   — see backend/CLAUDE.md for detail
cli/       oshihub — Rust CLI (clap + reqwest + tokio)
docs/      API_CONTRACT.md + diagrams/ (Mermaid sources for OVER_VIEW.html)
```

**`OVER_VIEW.html` is the architecture map** — every module and how it connects,
at file level. It's a browser page, so it's for the human; the sections below
are the text equivalent for working in here. Don't dedupe one against the
other, they have different readers.

## Read first — things that will bite you

**Bun must be canary (or stable ≥ 1.4).** Stable 1.3.x *crashes on startup*:
`mongoose` → `bson` 7.3.1 calls `node:v8`'s `startupSnapshot.isBuildingSnapshot()`
at import time, which throws `ERR_NOT_IMPLEMENTED`. This is why the Dockerfile
pins `oven/bun:canary`. Note `bun --version` reports the same number for canary
and stable, so it cannot be used to tell them apart — check behaviour instead:

```sh
bun -e 'console.log(process.getBuiltinModule("v8").startupSnapshot.isBuildingSnapshot())'
# false = fine; throws = too old
```

**The backend is deployed, and the CLI points at it by default.** Production is
`https://oshihub.195.201.149.64.sslip.io` (Hetzner, Docker, behind host Caddy;
ssh alias `hetzner`). The local config file `~/.config/oshihub/config.toml`
targets it, so **running `oshihub` hits production, not localhost**. For local
work: `OSHIHUB_API_URL=http://localhost:3000 oshihub <cmd>`.

**Local and deployed backends can now run simultaneously** (as of
`LIVE_DETECTION.md` Phase 1). Twitch EventSub moved from websocket to webhook
transport, so subscriptions point at `PUBLIC_URL` — the production callback —
rather than whichever backend most recently opened a connection. A local
backend's boot-time reconcile just confirms the same shared subscription set;
it never receives the actual webhook traffic. The next constraint still
applies regardless, though:

**MongoDB is Atlas, shared between local and production.** Local dev writes to
the same data the deployed backend serves. Atlas also rejects by *source IP*
before checking credentials, so new machines need adding under Network Access.

## Commands

```sh
# backend/
bun install
bun run dev                       # hot reload, port 3000
bunx tsc --noEmit                 # typecheck (src/routes/sync.test.ts has
                                  # pre-existing unrelated errors — ignore those)
docker build -t oshihub-backend . # container build

# cli/
cargo build
cargo test                        # unit tests only, no network
cargo test -- --ignored           # contract tests; hits the backend in config.rs
cargo run -- <command>

# docs/
python3 docs/diagrams/regen.py    # ONLY after editing a .mmd — see below
```

There is no test framework on the backend; `integration.test.ts` is a standalone
script that hits real APIs and the real database.

`cargo test -- --ignored` runs `cli/src/contract.rs`, which calls each endpoint
through the CLI's own serde types to catch backend drift. The frozen fixtures
in `models.rs`/`routes.rs` can only prove the types parsed the backend *on the
day each capture was taken*; these prove it still does. Shapes are recorded in
`docs/API_CONTRACT.md`. Target follows `config.rs`, so
`OSHIHUB_API_URL=http://localhost:3000 cargo test -- --ignored` checks local.

## CLI architecture

- `main.rs` — clap subcommands and all output formatting.
- `routes.rs` — one function per backend endpoint. Every request goes through
  `config::client()`; `read_body()` checks HTTP status *before* parsing, so a
  401 doesn't surface as a serde type error.
- `config.rs` — resolves `api_url` and `api_token` independently, each
  env var > `~/.config/oshihub/config.toml` > default. Caches a shared
  `reqwest::Client` in a `OnceLock` with the bearer token as a default header.
  **`external_client()` exists deliberately**: thumbnail fetches hit Twitch and
  YouTube CDNs and must *not* carry our token.
- `theme.rs` — all colouring **for the CLI**. The `colored` crate handles
  `NO_COLOR`/TTY detection itself, so call sites colour unconditionally. Add
  new colours here rather than inline, so a concept keeps one colour everywhere.
  Its TUI twin is `tui/theme.rs` — see below for why they can't be one file.
- `models.rs` — serde types for backend responses.
- `tui/` — the `oshihub tui` ratatui interface. See `TUI_DEVELOPMENT.md`.
- `watch.rs` — the `oshihub watch` poll loop. All the logic lives in a pure
  `apply(state, poll_result) -> (state, actions)`; the async loop is glue.
  Keep it that way — it's why the rules are testable without a network or a
  clock, and it's the seam a one-shot `oshihub check` would reuse.
- `notify.rs` — desktop notifications by shelling out to `notify-send`.

**Why `watch` exists, and why it polls.** Its value is *unifying* Twitch and
YouTube into one notification feed that needs no browser, no platform account,
and no bell toggle — not beating Twitch at delivering Twitch alerts, which it
can't (native push is instant; this adds up to one poll interval). That
tradeoff is accepted, so don't "fix" the latency. In particular **don't
rebuild this on SSE**: it was designed that way, and polling won because this
runs on a laptop — a pushed event fired while the lid is shut is gone, whereas
the next poll returns current truth. `Todo.md` records the full reversal. SSE
only becomes interesting if a TUI ever wants a genuine live feed, and
`markLive`'s edge boolean is the hook if so.

**`watch.rs`'s two non-obvious rules**, both mirroring the backend's
`scheduler.ts` guards: a failed poll must change *nothing* (a failed check and
a negative check are indistinguishable in the data, but only one means
"offline"), and eviction takes two consecutive misses. Streams are keyed on
`externalId` — `stream.url` is constant across a Twitch channel's streams and
`startTime` is rewritten on re-upsert, so both alternatives are wrong.

**`notify.rs` has three details that were verified against the running mako
daemon**, not reasoned about, and all three are load-bearing: `-a oshihub` is
mandatory (omarchy's mako config lets the default `notify-send` app name
through do-not-disturb), the body needs Pango escaping (`&` before `<`/`>`),
and `--wait` must come *before* the `--` terminator or notify-send counts it
as a third positional and rejects the call. Re-verify against the real daemon
if you change the argument list.

Thumbnails render via `viuer` (kitty/iTerm/Sixel, block fallback) for the
plain CLI (`lookup`/`live`). Placing text *beside* an image needs manual
`crossterm` cursor control — `viuer`'s `restore_cursor` only returns to the
position before drawing. **`viuer` cannot be used from inside the TUI**: it
writes graphics escapes straight at the cursor, while ratatui repaints every
cell each frame with no idea those cells are occupied. The TUI solves the
same problem with the `ratatui-image` crate instead (a real widget that
participates in ratatui's buffer) — see the TUI section below.

## TUI (`oshihub tui`)

Planned in `TUI_DEVELOPMENT.md`, which maps every CLI subcommand onto a phase
and carries the running checklist. Read it before touching `cli/src/tui/`.

- `tui/mod.rs` — terminal lifecycle (raw mode, alternate screen, panic hook)
  and the render loop.
- `tui/app.rs` — state, plus the `VtuberChannel` → `VtuberRow` mapper. The API
  DTO must not leak into the render layer.
- `tui/event.rs` — input handling.
- `tui/ui.rs` — render. **A pure function of `&App`; keep it that way.**
  Ratatui is immediate-mode: every tick throws away the last frame and redraws
  from current state, so there is no incremental update to reason about. That
  is what makes the model easy to keep correct.

**The panic hook restores the terminal *before* delegating to the default
hook**, and the order is the whole point. Panic output goes to stderr, which is
still aimed at the alternate screen — printing first paints the message onto a
buffer the terminal discards a moment later, so the user sees a silent exit
with no explanation. Verified by panicking on purpose with the hook on and off:
without it, ratatui's `Terminal::drop` shows the cursor but leaves the process
in the alternate screen *and* in raw mode.

**`theme.rs` and `tui/theme.rs` are deliberately separate.** `theme.rs` returns
`ColoredString`, which embeds raw ANSI escape bytes; written into a ratatui
buffer those render as literal text rather than colour. The TUI needs
`ratatui::style::Style` instead. They can't merge, so the convention is: the
same concept keeps the same colour in both, and both get updated together.

**Images use `ratatui-image`, not `viuer`.** `Picker::from_query_stdio`
(`App::picker`, probed once at startup — must run after `setup_terminal`
enters raw mode, since it queries stdio) detects terminal capability, and
`Picker::new_protocol` bakes a decoded image into a `Protocol` at a **fixed**
cell size — `ui.rs` must render it into a `Rect` of exactly that size, or the
image sits in one corner instead of filling/centering. `App::avatar` (Detail
header) and `App::thumbnail` (a focus-following preview pane for whichever
stream has focus, *not* per-row in the streams `List` — each row is one
`Line`, nowhere near tall enough for an image) each follow the same shape:
an independent background fetch, a `pending_*_id` staleness guard, `None` on
any failure rather than a surfaced error (avatars/thumbnails are cosmetic).
`ratatui-image` needs `default-features = false` — the default pulls in
`chafa-dyn`, a C-library ASCII-art backend needing `libchafa`/pkg-config that
this project has no use for.

## Auth

When `API_TOKEN` is set, `/api/*` and `/sync/*` require
`Authorization: Bearer <token>` (`backend/src/lib/require-api-token.ts`).
`/auth/*` is exempt — it's a browser redirect flow and can't carry a header.
The backend **refuses to start** if `NODE_ENV=production` and no token is set,
rather than quietly serving an open API.

## Conventions

- Commit subjects: `Add:` / `Fix:` / `Docs:` + imperative summary.
- **Never stage `.gitignore`** unless explicitly asked — the user keeps local
  edits to it deliberately uncommitted.
- Verify against the real running system rather than reasoning from API
  surface. Several bugs here were only found by reading crate source, inspecting
  image contents, or driving the actual binary.
- **The TUI is the one exception: the user reviews it by running it.** Stop at
  the end of each phase in `TUI_DEVELOPMENT.md`, say exactly what to run and
  which keys to press, and wait for confirmation before committing or starting
  the next phase. Don't write unit tests for render or navigation code, and
  don't script pty runs to self-verify — correctness there is how it looks and
  feels, which a keystroke replay confirms neither of. `cargo build` passing
  with no warnings is still required; a compile error is not an "obvious pass".
- Comments should explain *why*, especially where the code looks odd — most
  oddities here are load-bearing workarounds.

## Design docs (tracked)

- `OVER_VIEW.html` — the architecture map, ten sections of diagrams: system
  map, backend module graph, boot sequence, the live path, the sync path, the
  data model, CLI internals, the TUI message loop, endpoint reference, and a
  "who writes what" table. Self-contained (inlined SVG, no network, no JS) so
  it opens from disk. **Its `<svg>` blocks are generated** — edit
  `docs/diagrams/*.mmd` and run `regen.py`, never the HTML.
- `docs/diagrams/` — the nine Mermaid sources, the render configs, and
  `regen.py`. Its README explains why regenerating churns a 400KB diff even
  when nothing changed (mermaid-cli measures text in headless Chromium, so
  layout coordinates vary run to run), and why labels must use HTML entities.
- `docs/API_CONTRACT.md` — every endpoint's actual JSON shape, captured from
  the running production backend rather than read off the handlers: the
  Mongoose envelope, which fields are genuinely optional, dense-vs-sparse
  stats semantics, and the Rust type each maps to. Paired with
  `cli/src/contract.rs` (above) so drift fails a test instead of a user's CLI.
- `LIVE_DETECTION.md` — the plan the current live-detection stack was built
  from, in three phases: the shared `live-state.ts` writer, Twitch's move to
  webhook transport, and YouTube polling. All implemented. Read it for *why*
  things changed.
- `TWITCH_EVENTSUB.md` — reference for the Twitch half as it runs: webhook
  transport, app-token auth, the callback's signature-verification traps, the
  subscription-status table. Its appendix keeps the websocket-era lessons.
- `YOUTUBE_LIVE.md` — reference for the YouTube half: the two populations and
  why they need different mechanisms, quota math, why RSS is edge-detection
  only, and the failed-poll guards.
- `TUI_DEVELOPMENT.md` — the plan and running checklist for `oshihub tui`:
  every CLI subcommand mapped to a phase, the keymap, the verified traps
  (`Box<dyn Error>` isn't `Send`, `viuer` vs ratatui — resolved via
  `ratatui-image`, the two theme files), and the review-by-running
  convention. Phases 0–7 are done (v0.1); Phase 7.5's thumbnails are done,
  its general polish pass stays open-ended by design; Phase 8 (dashboard)
  is complete as of v0.5.0 — all three charts under `g`.

## Untracked local docs (gitignored, but read them)

- `Todo.md` — roadmap and design notes. Its YouTube live-detection research
  became `LIVE_DETECTION.md`, now fully implemented (all three phases).
- `bug-report.md` — bugs worth remembering. The duplicate `status: 'live'`
  Stream docs bug and EventSub's 5-Twitch-VTuber websocket ceiling are both
  now fixed (see `LIVE_DETECTION.md` Phases 0 and 1).
- `cli/IMAGE_RENDERING.md` — terminal image rendering writeup for the plain
  CLI's `viuer`-based avatar and per-stream thumbnails (`lookup`/`live`).
  Cross-references `TUI_DEVELOPMENT.md`'s Phase 7.5 for the TUI's separate
  `ratatui-image`-based solution to the same underlying problem.

## Known gaps

`oshihub watch` takes no lock, so a terminal instance plus the enabled
`oshihub-watch.service` user unit both notify (documented, not solved — a
lockfile would be the CLI's second disk write); this is also why the TUI
deliberately won't notify. The TUI itself, developed directly on `main`, is
**v0.1 complete**: Phases 0–7 ship (list, detail, live badges, search/filter,
sync/delete/create, edit, auto-refresh). Phase 7.5 has landed a VTuber avatar
in Detail's header and a focus-following stream thumbnail preview pane, both
via `ratatui-image` — List rows stay text-only for now, a deliberate scope
cut rather than a gap. Phase 8 (dashboard/charts) is complete as of
v0.5.0: `g` opens a per-VTuber dashboard — stream frequency and median
duration side by side, follower/subscriber trend line below — backed by
`GET /api/vtubers/:id/stats/{stream-frequency,subscriber-trend,duration-trend}`
and the backend stats-sync poller (see `backend/CLAUDE.md`). Details in
`TUI_DEVELOPMENT.md`.
