# VTuber Tracker

A polyglot project: a Hono/Bun REST API that aggregates VTuber data from HoloDex, YouTube, and Twitch into a unified MongoDB schema, plus `oshihub`, a Rust CLI client for it.

Track the streamers you watch from one place in the terminal — list them, look up recent streams and clips with inline thumbnails, see who's live right now, and jump straight to a channel in your browser.

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

### 4. Twitch live notifications (optional)

Twitch VTubers can be updated in real time via EventSub over WebSocket, rather than waiting for the next sync. This needs a one-time interactive authorization, because **EventSub's WebSocket transport requires a user access token**, not the app token used everywhere else.

1. In the [Twitch dev console](https://dev.twitch.tv/console), add `http://localhost:3000/auth/twitch/callback` as an OAuth Redirect URL on your application.
2. With the backend running, visit http://localhost:3000/auth/twitch/login and approve.

The token pair is stored in MongoDB (not `.env`) and refreshed automatically, so this is genuinely one-time. See [`TWITCH_EVENTSUB.md`](TWITCH_EVENTSUB.md) for the full design.

## CLI (`oshihub`)

### Commands

| Command | Alias | Description |
|---|---|---|
| `list` | `l` | List all tracked VTubers |
| `lookup <name>` | `lk` | Show a VTuber's live status, recent streams and clips, with thumbnails. `--limit <n>` caps how many of each (max 10) |
| `live` | `lv` | Show everyone who's currently live |
| `create <url>` | `c` | Register a VTuber from a channel URL. Parses platform (`youtube.com`/`youtu.be` → YouTube, `twitch.tv` → Twitch) and channel ID out of the URL |
| `jump <name>` | `j` | Open a VTuber's channel in your browser |
| `delete <name>` | `d` | Remove a VTuber and all their streams, clips, and stat snapshots |
| `sync <name>` | `s` | Force a refresh from the source API, bypassing the staleness gates |
| `config` | | Show the resolved backend URL and where it came from |

Name arguments are partial and case-insensitive.

### CLI configuration

Everything is optional — with no configuration the CLI talks to `http://localhost:3000` unauthenticated.

Settings resolve **environment variable → config file → default**:

| Setting | Env var | Config key | Default |
|---|---|---|---|
| Backend URL | `OSHIHUB_API_URL` | `api_url` | `http://localhost:3000` |
| Auth token | `OSHIHUB_API_TOKEN` | `api_token` | none |

The config file lives at `~/.config/oshihub/config.toml` (respects `XDG_CONFIG_HOME`):

```toml
api_url = "https://your-host.example.com"
api_token = "your-shared-secret"
```

`oshihub config` reports which source won — useful when a request unexpectedly 401s. Since each setting resolves independently, you can keep the URL in the file and the token in the environment.

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

There is no scheduler — sync only runs when a request hits one of these routes, or when Twitch EventSub reports that a tracked channel went live. Registering a VTuber triggers an initial sync automatically, so a new entry has streams and clips immediately.

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
- [ ] Update — `PUT /api/vtubers/:id` exists on the backend, no CLI command calls it yet
- [x] Delete
- [x] Force sync via `sync`
- [x] Configurable backend URL and auth token

Known gaps:

- No scheduler — live status is only current for Twitch (via EventSub) or after a manual `sync`.
- YouTube has no "went live" push equivalent to Twitch's EventSub; polling is the planned approach.
- A TUI dashboard is planned but not started.
