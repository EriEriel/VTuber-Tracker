# VTuber Tracker

A polyglot project: a Hono/Bun REST API that aggregates VTuber data from HoloDex, YouTube, and Twitch into a unified MongoDB schema, plus `oshihub`, a Rust CLI client for it.

Track the streamers you watch from one place in the terminal — list them, look up recent streams and clips with inline thumbnails, see who's live right now, and jump straight to a channel in your browser. One-shot subcommands or a full-screen [TUI](#tui-oshihub-tui) — same backend, either way.

```
.
├── backend/   Hono/Bun REST API (TypeScript)
└── cli/       oshihub — Rust CLI client (clap + reqwest)
```

The CLI talks to the backend over HTTP. It defaults to `http://localhost:3000`, so the backend must be running first — see [CLI configuration](#cli-configuration) to point it elsewhere.

## Requirements

- **[Bun](https://bun.com) — canary, or stable ≥ 1.4.** ⚠️ Stable 1.3.x **crashes on startup**: `mongoose` pulls in `bson` 7.3.1, which calls `node:v8`'s `startupSnapshot.isBuildingSnapshot()` at import time, and that throws `ERR_NOT_IMPLEMENTED` on 1.3.x. Install canary with `bun upgrade --canary`. Verify with:
  ```sh
  bun -e 'console.log(process.getBuiltinModule("v8").startupSnapshot.isBuildingSnapshot())'
  # must print `false` — if it throws, your Bun is too old
  ```
- **[Rust](https://rustup.rs)** (2024 edition) for the CLI.
- **MongoDB** — a [free Atlas cluster](https://www.mongodb.com/atlas) or a local `mongod`.
- API credentials for whichever platforms you want to track (see below).

A terminal with **kitty graphics protocol** support (Ghostty, Kitty, WezTerm) renders thumbnails inline. Anything else falls back to coloured blocks automatically — nothing breaks.

## Setup

### 1. API credentials

Only `MONGODB_URI` is always required; the rest depend on which platforms you care about.

| Variable | Needed for | Where to get it |
|---|---|---|
| `MONGODB_URI` | **always** | Atlas connection string, or `mongodb://localhost:27017/vtuber-tracker` |
| `HOLODEX_API_KEY` | YouTube VTubers listed on HoloDex (best metadata: org, sub-unit) | [holodex.net](https://holodex.net) → account settings |
| `YOUTUBE_API_KEY` | YouTube VTubers **not** on HoloDex, and YouTube stats | [Google Cloud Console](https://console.cloud.google.com) → enable *YouTube Data API v3* |
| `TWITCH_CLIENT_ID` / `TWITCH_CLIENT_SECRET` | Twitch VTubers | [dev.twitch.tv/console](https://dev.twitch.tv/console) → register an application |

If you're only tracking Twitch channels, you can leave the HoloDex and YouTube keys out entirely (and vice versa).

### 2. Backend

```sh
cd backend
bun install
```

Create `backend/.env`:

```sh
MONGODB_URI = mongodb+srv://user:pass@cluster.mongodb.net/vtuber-tracker
HOLODEX_API_KEY = your-key
YOUTUBE_API_KEY = your-key
TWITCH_CLIENT_ID = your-id
TWITCH_CLIENT_SECRET = your-secret
```

If you're using Atlas, add your machine's IP under **Network Access** — Atlas rejects connections by source IP before it ever checks credentials.

Then:

```sh
bun run dev
```

Open http://localhost:3000. You should see `MongoDB connected successfully` in the logs.

### 3. CLI

```sh
cd cli
cargo build --release
cargo install --path .     # optional: puts `oshihub` on your PATH
```

With the backend running, try:

```sh
oshihub create https://www.twitch.tv/tawffie
oshihub list
oshihub lookup tawffie
oshihub live
```

### 4. Live detection (optional)

Live status updates on its own, without waiting for a manual `sync`.

**Twitch** uses EventSub over **webhook** transport, which needs a publicly reachable HTTPS callback (port 443 — Twitch rejects anything else). Set both in `.env`:

```
EVENTSUB_SECRET=...    # openssl rand -base64 32
PUBLIC_URL=https://your-host.example.com
```

The backend reconciles its subscriptions on boot and Twitch then calls `POST /eventsub/callback`, which authenticates every request by HMAC signature. There's no interactive authorization step — subscription management uses the app token. Webhook transport also removed the old 5-VTuber ceiling that WebSocket transport imposed (`max_total_cost` 10 vs 10,000).

**YouTube** has no push equivalent, so the backend polls every 5 minutes and derives go-live/went-offline edges by diffing. Optional knobs:

```
YOUTUBE_POLL_INTERVAL_MS=300000   # default 5 minutes
YOUTUBE_POLL_DISABLED=true        # set on a second machine so it doesn't double-poll
```

Only run the poller in **one** place — both instances would spend YouTube quota against the same database. (Twitch is different: its subscriptions are bound to a shared callback URL, so a second backend converges harmlessly.)

Reference docs: [`TWITCH_EVENTSUB.md`](TWITCH_EVENTSUB.md) and [`YOUTUBE_LIVE.md`](YOUTUBE_LIVE.md) describe each integration as it runs; [`LIVE_DETECTION.md`](LIVE_DETECTION.md) is the design plan both came from.

## CLI (`oshihub`)

### Commands

| Command | Alias | Description |
|---|---|---|
| `list` | `l` | List all tracked VTubers |
| `lookup <name>` | `lk` | Show a VTuber's live status, recent streams and clips, with thumbnails. `--limit <n>` caps how many of each (max 10) |
| `live` | `lv` | Show everyone who's currently live |
| `watch` | `w` | Keep running and fire a desktop notification whenever someone goes live. `--interval <secs>` sets the poll gap (floor 15), `--notify-existing` notifies for whoever is already live instead of treating them as the baseline |
| `create <url>` | `c` | Register a VTuber from a channel URL. Parses platform (`youtube.com`/`youtu.be` → YouTube, `twitch.tv` → Twitch) and channel ID out of the URL |
| `jump <name>` | `j` | Open a VTuber's channel in your browser |
| `delete <name>` | `d` | Remove a VTuber and all their streams, clips, and stat snapshots |
| `sync <name>` | `s` | Force a refresh from the source API, bypassing the staleness gates |
| `config` | | Show the resolved backend URL and where it came from |
| `tui` | | Full-screen terminal interface — see [TUI](#tui-oshihub-tui) below |

Name arguments are partial and case-insensitive.

### CLI configuration

Everything is optional — with no configuration the CLI talks to `http://localhost:3000` unauthenticated.

Settings resolve **environment variable → config file → default**:

| Setting | Env var | Config key | Default |
|---|---|---|---|
| Backend URL | `OSHIHUB_API_URL` | `api_url` | `http://localhost:3000` |
| Auth token | `OSHIHUB_API_TOKEN` | `api_token` | none |
| `watch` poll gap | `OSHIHUB_WATCH_INTERVAL` | `watch_interval_secs` | `60` (floor 15) |
| Avatar icons on notifications | | `notify_icons` | `true` |
| Notification timeout (ms) | | `notify_timeout_ms` | `10000` (`0` = until dismissed) |

The config file lives at `~/.config/oshihub/config.toml` (respects `XDG_CONFIG_HOME`):

```toml
api_url = "https://your-host.example.com"
api_token = "your-shared-secret"
```

`oshihub config` reports which source won — useful when a request unexpectedly 401s. Since each setting resolves independently, you can keep the URL in the file and the token in the environment.

### Desktop notifications (`oshihub watch`)

`oshihub watch` polls the backend and fires a desktop notification each time a tracked VTuber goes live, on either platform. Clicking the notification opens the stream.

```sh
oshihub watch                      # poll every 60s
oshihub watch --interval 30
oshihub watch --notify-existing    # also notify for whoever is already live
```

#### Why not just use Twitch's and YouTube's own notifications?

For a single platform, you probably should — this isn't trying to beat Twitch at delivering Twitch alerts. The case for it is that they're *two separate systems*, neither of which you control:

- **One feed instead of two.** Twitch notifies through Twitch, YouTube through YouTube, each with its own settings, delivery quirks, and places to miss things. This is one stream of notifications for both, driven by the same roster you already manage with `create`/`list`.
- **No browser, no account, no bell.** Web push needs a browser running with the site permission granted; the apps need an account you're logged into and a notification toggle you remembered to set. `oshihub watch` is a ~4 MB resident process, and it notifies for anyone you've *registered* — whether you follow them is irrelevant. Useful if you'd rather not have an account, or not be logged into one.
- **It reports state, not intent.** The notification fires because the API says the stream is live. YouTube's bell in particular is well known for quietly not delivering, and neither platform tells you when it decided to skip one.
- **They're ordinary desktop notifications, so your rules apply.** Under mako, `[app-name=oshihub]` lets you restyle, group, or route them, and they honour do-not-disturb like anything else on the system. No browser tab has to be open for one to arrive.

**The honest tradeoff is latency, and it cuts both ways.** Twitch's own push is effectively instant, while this adds up to one poll interval on top (~30s average at the default), so for Twitch alone it is strictly slower. For YouTube the comparison inverts: the backend's detection floor is ~5 minutes, which in practice still tends to beat a bell notification that may arrive late or not at all.

It also only notifies while the machine is on and the watcher is running — there's no phone or push story here, deliberately. If you want to be reached when you're away from your desk, the platforms' own apps are the right tool and this isn't a replacement for them.

Whoever is live when it starts is treated as the quiet baseline and printed to the terminal rather than notified about — otherwise starting it would fire a burst of popups for streams you already knew about. `--notify-existing` opts out of that, which is also the easiest way to check notifications work without waiting for someone to go live.

Requires `notify-send` (from `libnotify`) and a notification daemon. Without either, it degrades to terminal output and says so once rather than failing.

Avatars are cached to `~/.cache/oshihub/avatars/` (the only thing the CLI ever writes to disk); set `notify_icons = false` to disable.

**To have it start with your session**, create `~/.config/systemd/user/oshihub-watch.service`:

```ini
[Unit]
Description=oshihub live VTuber notifications
PartOf=graphical-session.target
After=graphical-session.target

[Service]
Type=simple
ExecStart=%h/.cargo/bin/oshihub watch
Restart=on-failure
RestartSec=30

[Install]
WantedBy=graphical-session.target
```

Then `systemctl --user daemon-reload && systemctl --user enable --now oshihub-watch`, and read its output with `journalctl --user -u oshihub-watch -f`.

`ExecStart` points at the *installed* binary, so re-run `cargo install --path cli` and `systemctl --user restart oshihub-watch` after changing the CLI — otherwise the service keeps running the old build.

A bad token exits non-zero rather than retrying, so `Restart=on-failure` won't loop on it. Note there's no locking: a terminal instance *and* an enabled service means two watchers and duplicate notifications.

### TUI (`oshihub tui`)

```sh
oshihub tui
```

A full-screen `ratatui` interface over the same backend the one-shot commands use — **v0.1 complete**: main list, detail view (streams/clips), live badges that auto-refresh in place, incremental search/filter, and sync/delete/create/edit, all without leaving the terminal. It's a second front end alongside the one-shot commands above, not a replacement for them.

```
j / k / ↓ / ↑   move            Enter   detail view
/               filter          o       open channel in browser / stream
L               live-only       s       sync selected
?               help overlay    d       delete selected (confirms)
Esc / h         back            a       add from URL
q               quit            e       edit selected
```

Live badges refresh on their own, on the same `watch_interval_secs` config `oshihub watch` uses — a VTuber going live mid-session picks up the badge within one interval, no restart or manual `s` needed, and briefly highlights to show it's new.

Thumbnails aren't rendered inside the TUI yet — `viuer` writes graphics escapes straight at the cursor, which conflicts with ratatui repainting every cell each frame; see [`TUI_DEVELOPMENT.md`](TUI_DEVELOPMENT.md) for the full phase-by-phase design history and the trap notes. A dashboard/charts screen (Phase 8) is planned but deferred.

### Stack

- `clap` — CLI argument parsing
- `reqwest` — HTTP client
- `serde` / `serde_json` — JSON (de)serialization
- `tokio` — async runtime
- `viuer` / `image` — inline terminal thumbnails (kitty/iTerm/Sixel, with a block-character fallback)
- `crossterm` — cursor control, for placing thumbnails beside text
- `toml` / `dirs` — config file
- `open` — opens URLs in the default browser

## Deployment

The backend is containerized and expects to sit behind a reverse proxy.

```sh
cd backend
docker build -t oshihub-backend .
docker run -d --name oshihub --restart unless-stopped \
  -p 127.0.0.1:3001:3000 \
  -e API_TOKEN="$(openssl rand -base64 32)" \
  -v "$PWD/.env:/app/.env:ro" \
  oshihub-backend
```

Notes:

- **Mount `.env`, don't use `--env-file`.** Docker's `--env-file` can't parse `KEY = VALUE` with spaces around the `=`; Bun can.
- **`API_TOKEN` enables authentication.** When set, every `/api/*` and `/sync/*` request must send `Authorization: Bearer <token>`. Set the matching `api_token` in the CLI config.
- **It fails closed.** The image sets `NODE_ENV=production`, and the backend refuses to start in production without `API_TOKEN` rather than quietly serving an open API. Every route mutates data or spends a rate-limited API quota, so this is not optional in public deployments.
- **Bind to loopback** (`127.0.0.1:3001`) and let the reverse proxy terminate TLS. The bearer token is sent in plaintext, so it needs HTTPS to mean anything.

A minimal [Caddy](https://caddyserver.com) site block:

```caddyfile
your-host.example.com {
	reverse_proxy 127.0.0.1:3001
}
```

`PORT` overrides the listening port (default `3000`).

## Backend behavior

### Sync behavior

`POST /api/sync/{holodex,youtube,twitch,all}` does **not** force a refresh by default. Each VTuber is checked against two independent staleness gates before any external API call is made:

- **Live status** — skipped if `lastLiveSyncedAt` is under 15 minutes old
- **Stats** — skipped if `lastStatsSyncedAt` is under 24 hours old

These are evaluated separately, so a single call can refresh live status while skipping stats (or vice versa), depending on which gate has expired. Pass `?force=true` to bypass both gates unconditionally.

Full syncs (stats, clips, VOD backfill) run only when a request hits one of these routes. Registering a VTuber triggers an initial sync automatically, so a new entry has streams and clips immediately.

Live status is separate and does update on its own — Twitch pushes it via EventSub, and the YouTube poller derives it every 5 minutes (see [Live detection](#4-live-detection-optional)). Both write through the same `markLive`/`markEnded` pair, which is the only thing in the backend that sets a stream live or ends it.

### Twitch channel name → ID resolution

Twitch identifies channels by login name (e.g. `tawffie`), which can change, unlike YouTube's stable channel IDs. So the login name is only used once, at registration:

- `POST /api/vtubers` with `{ platform: "twitch", channelId: "<login name>" }` triggers `resolveTwitchUser()` (`src/lib/sync.ts`), which calls Twitch Helix's `GET /helix/users?login=` to resolve the login to a numeric user ID.
- That numeric ID — not the login — is stored as `platformChannelId` on the VTuber document.

Every sync afterward (`syncFromTwitch`) uses only the numeric ID for Twitch API calls. The login name is never looked up again, so a later Twitch username change doesn't break syncing.

### YouTube channel handle/URL resolution

`POST /api/vtubers` for `platform: "youtube"` accepts a literal channel ID (e.g. `UC1DCedRgGHBdm81E1llLhOQ`), a bare handle (`@holoen_raorapanthera`), or a full channel URL (`https://www.youtube.com/@holoen_raorapanthera`).

- `extractYoutubeHandle()` (`src/lib/sync.ts`) normalizes a full URL down to the bare `@handle` before either external API is queried — HoloDex's `/channels/{id}` endpoint accepts a bare handle but not a full URL.
- HoloDex is tried first using the normalized input. If it resolves, the VTuber is stored with `source: 'holodex'`.
- If HoloDex doesn't have the channel, `resolveYoutubeHandle()` falls back to the YouTube Data API's `forHandle` param — a direct lookup (not `search.list`), so it stays within the 10,000 units/day quota. The VTuber is stored with `source: 'youtube_api'`.

In both cases, `platformChannelId` is set from the **canonical channel ID returned by the API** (`data.id` / `resolved.id`), never the raw handle/URL string — so a handle or URL is only ever used to look the channel up once, the same way Twitch login names are.

### Profile URL resolution

`GET /api/vtubers/:id/profile-url` returns a browsable channel URL for a VTuber, computed differently per platform:

- **YouTube** — built directly from the stored `platformChannelId`: `https://youtube.com/channel/{platformChannelId}`. No external call needed, since that ID is already canonical.
- **Twitch** — the stored `platformChannelId` is only the numeric ID (see above), and Twitch channel pages only resolve by login name, never by ID. So this route calls `fetchTwitchUserById()` (`src/lib/sync.ts`) to resolve the numeric ID to its *current* login via Helix's `GET /helix/users?id=`, then builds `https://twitch.tv/{login}`. This is resolved fresh on every call rather than cached/stored, so it stays correct even after a Twitch username change.

This route exists so CLI commands like `jump` never need platform-specific URL logic of their own — they just ask the backend for a URL and open it.

## Status

CLI coverage of the backend:

- [x] Create: `POST /api/vtubers` via `create`
- [x] Read: list all VTubers via `list`
- [x] Read: search by name, with streams/clips/live status via `lookup`
- [x] Read: currently-live VTubers via `live`
- [x] Update: `PUT /api/vtubers/:id` via the TUI's `e` edit modal (no one-shot `update` subcommand)
- [x] Delete
- [x] Force sync via `sync`
- [x] Configurable backend URL and auth token
- [x] Desktop notifications on going live via `watch`
- [x] Full-screen TUI (`tui`) — v0.1 complete: list, detail, live, search/filter, create/sync/delete/edit, auto-refresh

Known gaps:

- No one-shot `update` subcommand — `PUT /api/vtubers/:id` is only reachable through the TUI.
- `watch` takes no lock, so a terminal instance and an enabled systemd service will both notify; same is true of two TUI sessions polling live status independently.
- TUI thumbnails and a dashboard/charts screen (Phase 8) are planned but not started — see [`TUI_DEVELOPMENT.md`](TUI_DEVELOPMENT.md).
