# API contract

What the backend actually sends, and what the Rust client actually requires.
`OVER_VIEW.html` section 9 lists *what each endpoint does*; this document is
the other half — the JSON shapes on the wire.

Every example below was captured from production
(`https://oshihub.195.201.149.64.sslip.io`) on **2026-07-27**, against a
database of 21 tracked VTubers, and trimmed for length. Nothing here is
inferred from reading the route handlers.

## Why this document exists

There is no schema shared between the backend and the CLI. `backend/src/models/`
defines the shapes in Zod and Mongoose; `cli/src/models.rs` and the structs in
`cli/src/routes.rs` re-declare them in serde. Nothing checks that the two still
agree — **the client mirrors the server's contract, and the mirror can go stale
silently.**

The CLI's existing unit tests do not close this. They deserialize *frozen
captures* (`cli/src/models.rs` fixtures dated 2026-06-27,
`cli/src/routes.rs` fixtures dated 2026-07-26). They prove the CLI can parse
what the backend returned on those dates. If the backend's shape drifted
afterwards, those tests still pass and the failure lands on a user's machine as
a serde error.

That is what `cli/src/contract.rs` is for — see [Drift check](#drift-check).

## Auth

`/api/*` and `/sync/*` require `Authorization: Bearer <API_TOKEN>` whenever the
backend has `API_TOKEN` set. `/eventsub/*` is exempt (Twitch cannot carry the
header) and relies on HMAC signature verification instead.

```
401 → {"error":"Unauthorized"}
```

The token is compared as `sha256(provided)` vs `sha256(expected)` through
`timingSafeEqual` — hashed first because `timingSafeEqual` throws on a length
mismatch, which would itself leak the real token's length.

## Conventions that apply everywhere

Documents come straight from Mongoose, so every one carries its envelope:

| Field | Type | Note |
|---|---|---|
| `_id` | string | 24-char hex ObjectId. The Rust side renames it to `id`. |
| `__v` | number | Mongoose version key. Modelled as `version: u32` on `VtuberChannel`. |
| `createdAt` / `updatedAt` | ISO 8601 string | Present on VTuber, Stream and Clip. **Absent on StatSnapshot** — that schema sets `timestamps: false`. |

Dates are ISO 8601 strings with milliseconds and a `Z` suffix
(`2026-07-26T03:09:00.000Z`). **The CLI never parses them.** It carries no date
crate, so every date it displays is either shown verbatim or truncated to the
first 10 characters. Any date arithmetic the client needs is done by the
backend and sent pre-computed (see the stats endpoints).

Error bodies are uniform:

```json
{ "error": "human-readable summary", "detail": "optional stringified cause" }
```

## `GET /api/vtubers`

Query params (all optional): `platform`, `org`, `isTracked`, `name`.
`name` is a case-insensitive substring match over both `name` and `englishName`,
with regex metacharacters escaped before it reaches `$regex`.

Returns a bare **array** (not an envelope), sorted by `name` ascending.

```json
[
  {
    "_id": "6a6538f2f227be73d5c4f3e0",
    "name": "Nimi Nightmare",
    "englishName": "Nimi Nightmare",
    "photo": "https://yt3.ggpht.com/...=s800-c-k-c0x00ffffff-no-rj",
    "platform": "youtube",
    "source": "holodex",
    "platformChannelId": "UCIfAvpeIWGHb0duCkMkmm2Q",
    "org": "Phase Connect",
    "suborg": "...",
    "isTracked": true,
    "lastSyncedAt": "2026-07-27T02:11:04.883Z",
    "lastLiveSyncedAt": "2026-07-27T02:11:04.883Z",
    "lastStatsSyncedAt": "2026-07-27T02:11:04.883Z",
    "createdAt": "2026-07-26T00:41:22.911Z",
    "updatedAt": "2026-07-27T02:11:05.402Z",
    "__v": 0
  }
]
```

### Required vs optional — the part that breaks the client

`cli/src/models.rs::VtuberChannel` declares these as **non-`Option`**, so a
missing *or* `null` value is a hard deserialize failure, not a degraded render:

`_id`, `__v`, `createdAt`, `updatedAt`, `englishName`, `isTracked`, `name`,
`photo`, `platform`, `platformChannelId`, `source`

These are `Option` and safe to omit: `org`, `suborg`, `lastSyncedAt`,
`lastLiveSyncedAt`, `lastStatsSyncedAt`.

> **Verified 2026-07-27:** all 21 production records have every required field
> present and non-null. `org`/`suborg` are absent on 20 of 21 — only the single
> `holodex`-sourced VTuber has them, because HoloDex is the only source that
> knows about agencies. Twitch and the YouTube Data API have no equivalent
> concept.

The risk this creates is worth stating plainly: `photo` and `englishName` are
required by the client but only ever *populated* by a successful sync. A record
created while an upstream API was misbehaving could carry `""` (which parses
fine) — but a schema change making either nullable would break every CLI
command at once.

Enum values are lowercase and must match exactly:

- `platform`: `youtube` | `twitch`
- `source`: `holodex` | `youtube_api` | `twitch_api`

## `GET /api/vtubers/live`

Returns a bare array of `{ vtuber, stream }` pairs, newest stream first.
`vtuber` is the full document above. `stream` is a **hand-built projection**,
not the Stream document:

```json
[
  {
    "vtuber": { "...full VTuber document..." },
    "stream": {
      "externalId": "317728559192",
      "title": "First Playthrough.. Hay-deez...nuts? !throne !clip",
      "url": "https://www.twitch.tv/tawffie",
      "thumbnailUrl": "https://static-cdn.jtvnw.net/previews-ttv/live_user_tawffie-640x360.jpg",
      "startTime": "2026-07-26T03:09:00.000Z"
    }
  }
]
```

Only these five fields are sent — no `_id`, no `status`, no `duration`.

**`externalId` is the field that matters.** It is the only stable per-stream
identity: `url` is `twitch.tv/{login}`, identical for every stream that channel
ever does, and `startTime` is rewritten on every `markLive` upsert. `oshihub
watch` keys its "went live" edge detection on it, so a client that fell back to
`url` would notify once per channel forever and never again.

Both `externalId` and `startTime` are `Option` on the Rust side
(`routes.rs::LiveStreamInfo`) purely for version skew — a CLI updated ahead of
a backend redeploy degrades to a coarser dedup key rather than failing to
parse. Do not read that as "these are optional in the response."

## `GET /api/vtubers/:id`

Returns an envelope with four keys. Related collections are capped at the 10
newest each.

```json
{
  "vtuber":    { "...full VTuber document..." },
  "streams":   [ "...up to 10, newest startTime first..." ],
  "clips":     [ "...up to 10, newest createdAt first..." ],
  "snapshots": [ "...up to 10, newest capturedAt first..." ]
}
```

`404 → {"error":"VTuber not found"}` for both a missing and a malformed id.

**Stream** documents carry every schema field:
`_id`, `__v`, `vtuberId`, `externalId`, `title`, `platform`, `startTime`,
`endTime`, `duration`, `status`, `url`, `thumbnailUrl`, `sourceApi`,
`createdAt`, `updatedAt`.

- `status`: `upcoming` | `live` | `ended` | `unknown`
- `duration`: seconds, or `null`. Observed in production: 10 of 210 streams
  `null`, 8 more exactly `0`.
- `endTime`: `null` while live.

**Clip**: `_id`, `__v`, `vtuberId`, `sourceStreamId`, `externalId`, `title`,
`url`, `viewCount`, `sourceApi`, `createdAt`, `updatedAt`.
`sourceStreamId` is `null` for every HoloDex clip — only the Twitch path
resolves a parent stream.

**StatSnapshot**: `_id`, `__v`, `vtuberId`, `subscriberCount`, `viewCount`,
`capturedAt`, `sourceApi`. **No `createdAt`/`updatedAt`** — this is the one
schema with `timestamps: false`.

The Rust side deliberately under-models this endpoint: `routes.rs::VtuberDetail`
declares only `streams` and `clips`, and `StreamInfo` only
`title`/`status`/`url`/`thumbnailUrl`. Serde ignores undeclared fields, so this
is safe by construction — adding a backend field can never break the client,
only removing or renaming one it declares.

## `GET /api/vtubers/:id/profile-url`

```json
{ "url": "https://twitch.tv/kribbyvt" }
```

For YouTube this is pure string construction from the stored channel id. For
Twitch it costs a live Helix call — the stored `platformChannelId` is the
numeric user id, and only Helix can translate it back to the login name a URL
needs. `502` if that lookup fails.

## The three stats endpoints

All three back the TUI dashboard. They share one design rule: **the backend
does all calendar arithmetic**, because the CLI carries no date crate. Buckets
are UTC, weeks start Monday (matching `$dateTrunc`'s `startOfWeek`).

### `GET /api/vtubers/:id/stats/stream-frequency`

Query: `unit=week|day` (default `week`), `buckets=<n>` (default 52, max 366).

```json
{
  "unit": "week",
  "from": "2025-08-04T00:00:00.000Z",
  "firstStreamAt": "2026-06-16T03:00:02.000Z",
  "counts": [null, null, "...", 8, 8, 8, 0],
  "starts": ["2025-08-04", "2025-08-11", "..."]
}
```

`counts` and `starts` are **dense and parallel**, oldest → newest, always
exactly `buckets` long. The three-way distinction in `counts` is the whole
point:

| Value | Means |
|---|---|
| `null` | Bucket ended before `firstStreamAt` — *not yet tracking*, render as absent |
| `0` | Tracked and genuinely quiet |
| `n` | `n` streams started in this bucket |

Only `live` and `ended` streams are counted. `upcoming` would put phantom counts
in future buckets (HoloDex returns scheduled streams — 6 were present in the
production sample), and `unknown` is unvetted.

The final bucket is the *current, partial* week — expect it to read low. The
TUI renders it muted for that reason.

### `GET /api/vtubers/:id/stats/subscriber-trend`

Query: `days=<n>` (default 90, max 365).

```json
{
  "from": "2026-04-28",
  "days": 90,
  "points": [ { "day": 87, "date": "2026-07-25", "subscribers": 713000 } ],
  "current": 713000,
  "delta7d": null,
  "delta30d": null
}
```

Unlike `stream-frequency`, `points` is **sparse**. A missing day means "didn't
sync", never "zero subscribers", and the chart connects across the gap. `day` is
an integer offset from `from` so gaps stay proportional on the x-axis without
the client touching a calendar.

One point per UTC day, the day's **last** snapshot winning — forced syncs
(a Twitch `stream.online`, a manual sync) can capture several in a day and the
newest is simply the freshest truth.

`delta7d`/`delta30d` are "current vs the newest snapshot at least N days old",
not an interpolation, and **`null` when history is too short to answer
honestly**. Verified in production: a channel with only 2 snapshots returns
`current: 713000` with both deltas `null` rather than a misleading `+0`.

### `GET /api/vtubers/:id/stats/duration-trend`

Query: same as `stream-frequency`, and the buckets line up column-for-column
with it by design so the two charts can sit side by side.

```json
{
  "unit": "week",
  "from": "2025-08-04T00:00:00.000Z",
  "starts": ["2025-08-04", "..."],
  "medians": [14032, 13167, 10499, 14042, 9616, null],
  "counts":  [5, 5, 5, 3, 5, 0],
  "overallMedian": 12920,
  "longest": 19194
}
```

Seconds throughout. `medians` is dense and parallel to `starts`; `null` means no
qualifying stream in that bucket, which is not the same statement as "0 hours".
`counts` is how many streams fed each median, so a client can tell a
pre-tracking bucket from a quiet one.

**Median, not mean, and floored at 600 seconds.** Both guards are load-bearing:

- Buckets hold 3–8 streams, where one 12-hour subathon would double a mean.
- YouTube-sourced Stream docs include Shorts and ordinary uploads. Verified in
  production: **15 of 200 streams with a duration are under 10 minutes, the
  shortest being 14 seconds, and 10 of those 15 are `youtube_api`-sourced.**
  Without the floor these would drag every median down.

Only `status: 'ended'` streams count — a running stream has no final duration —
and `$gte` also excludes the `null` durations left by EventSub-ended streams no
VOD sync has backfilled yet.

## `POST /api/sync/{holodex,youtube,twitch}`

Query: `?id=<vtuberId>` (optional, otherwise every tracked VTuber for that
source), `?force=true` (bypasses both staleness gates).

```json
{
  "source": "holodex",
  "results": [
    { "vtuberId": "...", "status": "success", "synced": { "live": true, "stats": false } },
    { "vtuberId": "...", "status": "skipped", "reason": "data is fresh" },
    { "vtuberId": "...", "status": "failed",  "error": "..." }
  ]
}
```

`status` is one of `success` | `skipped` | `failed`, and **a per-VTuber
`failed` still returns HTTP 200** — the array is a report, not an error
channel. A 500 means the whole call failed (usually a missing API key).

## `POST /api/sync/all`

Different shape from the three above — an object keyed by source, not a flat
array:

```json
{
  "summary": {
    "holodex": { "ok": true,  "count": 1,  "details": [ "...as above..." ] },
    "youtube": { "ok": true,  "count": 9,  "details": [] },
    "twitch":  { "ok": false, "error": "..." }
  }
}
```

Runs all three through `Promise.allSettled`, so one source's API being down
cannot cost the other two their sync.

## `POST /api/vtubers`

```json
{ "platform": "youtube", "channelId": "@holoen_raorapanthera" }
```

`channelId` accepts a channel id, an `@handle`, or a full channel URL — the CLI
sends whatever it parsed out of the URL you gave it and lets the backend
resolve. `201` on success with `{ message, vtuber }`; `409` with the existing
record if already registered; `404` if the channel does not resolve; `502` if
the upstream lookup itself failed.

Registration only establishes identity. Streams and clips arrive moments later
from a **fire-and-forget forced sync** the response does not wait for, so an
immediate `GET /api/vtubers/:id` can legitimately come back with empty arrays.

## `PUT /api/vtubers/:id`

All fields optional: `name`, `englishName`, `photo`, `isTracked`, `org`,
`suborg`. Returns `{ message, vtuber }`.

The CLI sends `org`/`suborg` as `null` rather than `""` when cleared — the
backend distinguishes an absent org from a set one, so an empty-string org
would be a third, meaningless state.

## `DELETE /api/vtubers/:id`

Returns `{ message, vtuber }` with the deleted document. Cascades to Stream,
Clip and StatSnapshot, and unsubscribes EventSub for Twitch channels.

## Rust type map

| Endpoint | Rust type | File |
|---|---|---|
| `GET /api/vtubers` | `Vec<VtuberChannel>` | `models.rs` |
| `GET /api/vtubers/live` | `Vec<LiveEntry>` | `routes.rs` |
| `GET /api/vtubers/:id` | `VtuberDetail` (partial) | `routes.rs` |
| `…/profile-url` | `ProfileUrlResponse` | `routes.rs` |
| `…/stats/stream-frequency` | `StreamFrequency` | `routes.rs` |
| `…/stats/subscriber-trend` | `SubscriberTrend` | `routes.rs` |
| `…/stats/duration-trend` | `DurationTrend` | `routes.rs` |
| `POST /api/sync/*`, `PUT`, `DELETE` | *(body discarded)* | `routes.rs` |

Every response goes through `routes.rs::read_body`, which checks HTTP status
**before** handing anything to serde. Without it, an error-shaped body
(`{"error": "..."}`) reaches a `Vec<T>` deserializer and surfaces as
`invalid type: map, expected a sequence` — technically true and useless for
diagnosing what is almost always a missing token.

## Drift check

`cli/src/contract.rs` holds `#[ignore]`d tests that fetch from the configured
backend and deserialize the real responses into the real types. They are
ignored so plain `cargo test` stays offline, as `CLAUDE.md` promises:

```sh
cd cli
cargo test                      # unit tests only, no network (unchanged)
cargo test -- --ignored         # hits the backend in ~/.config/oshihub/config.toml
OSHIHUB_API_URL=http://localhost:3000 cargo test -- --ignored   # against local
```

These are the only tests in the repo that can catch backend drift, because they
use the same serde types the binary does rather than a second description of
them. Run them after any change to `backend/src/models/` or to a route's
response shape.
