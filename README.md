To install dependencies:
```sh
bun install
```

To run:
```sh
bun run dev
```

open http://localhost:3000

## Sync behavior

`POST /api/sync/{holodex,youtube,twitch,all}` does **not** force a refresh by default. Each VTuber is checked against two independent staleness gates before any external API call is made:

- **Live status** — skipped if `lastLiveSyncedAt` is under 15 minutes old
- **Stats** — skipped if `lastStatsSyncedAt` is under 24 hours old

These are evaluated separately, so a single call can refresh live status while skipping stats (or vice versa), depending on which gate has expired. Pass `?force=true` to bypass both gates unconditionally.

There is no scheduler — sync only runs when a request hits one of these routes.

## Twitch channel name → ID resolution

Twitch identifies channels by login name (e.g. `tawffie`), which can change, unlike YouTube's stable channel IDs. So the login name is only used once, at registration:

- `POST /api/vtubers` with `{ platform: "twitch", channelId: "<login name>" }` triggers `resolveTwitchUser()` (`src/lib/sync.ts`), which calls Twitch Helix's `GET /helix/users?login=` to resolve the login to a numeric user ID.
- That numeric ID — not the login — is stored as `platformChannelId` on the VTuber document.

Every sync afterward (`syncFromTwitch`) uses only the numeric ID for Twitch API calls. The login name is never looked up again, so a later Twitch username change doesn't break syncing.

## YouTube channel handle/URL resolution

`POST /api/vtubers` for `platform: "youtube"` accepts a literal channel ID (e.g. `UC1DCedRgGHBdm81E1llLhOQ`), a bare handle (`@holoen_raorapanthera`), or a full channel URL (`https://www.youtube.com/@holoen_raorapanthera`).

- `extractYoutubeHandle()` (`src/lib/sync.ts`) normalizes a full URL down to the bare `@handle` before either external API is queried — HoloDex's `/channels/{id}` endpoint accepts a bare handle but not a full URL.
- HoloDex is tried first using the normalized input. If it resolves, the VTuber is stored with `source: 'holodex'`.
- If HoloDex doesn't have the channel, `resolveYoutubeHandle()` falls back to the YouTube Data API's `forHandle` param — a direct lookup (not `search.list`), so it stays within the 10,000 units/day quota. The VTuber is stored with `source: 'youtube_api'`.

In both cases, `platformChannelId` is set from the **canonical channel ID returned by the API** (`data.id` / `resolved.id`), never the raw handle/URL string — so a handle or URL is only ever used to look the channel up once, the same way Twitch login names are.
