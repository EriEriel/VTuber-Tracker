# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

See the repo-root `CLAUDE.md` for cross-cutting concerns: the Bun canary requirement, the deployed instance, and why only one backend may run at a time.

## Commands

```sh
bun run dev                          # start server with hot reload on port 3000
bun run src/integration.test.ts      # run manual integration test (hits real APIs + DB)
bunx tsc --noEmit                    # typecheck
docker build -t oshihub-backend .    # container build
```

There is no build step — Bun runs TypeScript directly. No test framework is configured; `integration.test.ts` is a standalone script. `bunx tsc --noEmit` reports pre-existing unrelated errors in `src/routes/sync.test.ts`; ignore those.

## Architecture

A Hono/Bun REST API that aggregates VTuber data from three external APIs into a unified MongoDB schema.

**Entry:** `src/index.ts` mounts three real route groups (`vtubersRoute`, `syncRoute`, `eventsubRoute`) and three dev/debug test routes (`holodex.test`, `youtube.test`, `twitch.test`), then connects the DB, reconciles EventSub subscriptions once, and serves on `process.env.PORT ?? 3000`.

> **Do not add `export default app`.** Bun's entry wrapper calls `Bun.serve()` on the entry module's default export if it looks like a server config — and a Hono instance does, since it has `fetch`. Combined with the explicit `Bun.serve()` call, that binds the port twice and dies with `EADDRINUSE`. `bun run --hot` skips that wrapper, so this only fails *outside* dev (e.g. in the container). There's a comment in `index.ts` guarding this.

**Routes:**
- `src/routes/vtubers.ts` — CRUD for VTuber documents; POST registration resolves the source API automatically and fires an initial sync. Also `GET /api/vtubers/live`, registered **before** `GET /api/vtubers/:id` so `live` isn't captured as an `:id` param.
- `src/routes/sync.ts` — triggers sync for `POST /api/sync/{holodex,youtube,twitch,all}` with optional `?id=` and `?force=true`
- `src/routes/eventsub.ts` — `POST /eventsub/callback`, Twitch's EventSub webhook (see EventSub below)

**Sync layer (`src/lib/sync.ts`):** Three source-specific functions (`syncFromHolodex`, `syncFromYoutube`, `syncFromTwitch`). Each enforces two staleness gates per VTuber before hitting external APIs:
- `lastLiveSyncedAt` — skip if fresher than 15 minutes
- `lastStatsSyncedAt` — skip if fresher than 24 hours
- `?force=true` bypasses both gates

Registering a VTuber calls the matching sync function fire-and-forget (not awaited, so a slow sync can't fail registration), which is why a new entry has streams and clips immediately.

**Models (`src/models/`):** Each model file exports both a Zod schema (for input validation) and a Mongoose model. All four content collections share a `sourceApi` discriminator field.

**DB (`src/lib/db.ts`):** Singleton connection with race-condition protection — a single `connectionPromise` prevents concurrent `mongoose.connect()` calls during server startup. Logs `error.message` only, never the error object: Mongoose attaches a full `TopologyDescription` per replica-set member, which under a restarting container turns one failure into screenfuls. It still throws (the tests need that); `index.ts` catches and exits 1.

**Twitch tokens:** `src/lib/twitch-token.ts` — **app** access token (client credentials), in-memory cache with a 60-second expiry buffer. Used for every Twitch Helix call in the codebase, including EventSub subscription management — webhook transport doesn't need a user token the way websocket transport did. (That user-token machinery, `twitch-user-token.ts` + the `TwitchAuth` model + `routes/auth.ts`, was deleted when EventSub moved to webhooks; see `LIVE_DETECTION.md` Phase 1.)

## Twitch EventSub

`src/lib/twitch-eventsub.ts` manages EventSub **webhook** subscriptions (not websocket — that hit a `max_total_cost` of 10, capping live tracking at 5 Twitch VTubers; webhook's cap is 10,000). It holds no persistent connection: `initTwitchEventSub()` runs once at boot to reconcile subscription coverage against tracked Twitch VTubers, and Twitch delivers events by calling `POST /eventsub/callback` (`src/routes/eventsub.ts`) whenever it wants, for as long as the subscription exists.

On `stream.online`, `handleNotification()` calls `syncFromTwitch(id, true)` rather than building a Stream from the event payload (intentionally minimal — no title, no thumbnail). On `stream.offline` it calls `markEnded()` (`src/lib/live-state.ts`) directly, since that event carries even less (no timestamp at all).

Subscription coverage is keyed on `(broadcasterId, type)` pairs. Status handling matters here: `webhook_callback_verification_pending` is mid-handshake and must **not** be deleted on reconcile (it's about to become `enabled` on its own); `webhook_callback_verification_failed` and `notification_failures_exceeded` are terminal and must be deleted and recreated. Both `enabled` and `..._pending` count as coverage, so reconcile doesn't race a duplicate create against an in-flight handshake.

Because subscriptions are bound to the callback URL rather than a connection/session, they **survive backend restarts** — the boot-time reconcile self-heals drift but is no longer load-bearing the way session-bound websocket subscriptions were.

`routes/eventsub.ts` verifies every incoming request's HMAC signature (`Twitch-Eventsub-Message-Signature`, computed over `message-id + timestamp + raw body` — must read the raw body via `c.req.text()` **before** any `JSON.parse`, or the HMAC never matches) via `timingSafeEqual`, rejects timestamps older than 10 minutes, and dedupes `Twitch-Eventsub-Message-Id` in an in-memory `Map` (Twitch retries on any non-2xx response). Notifications get a `204` immediately, then `handleNotification()` runs fire-and-forget — `stream.online`'s `syncFromTwitch()` fan-out routinely exceeds Twitch's 10-second response budget (see `bug-report.md` #10).

`TWITCH_EVENTSUB.md` at the repo root is the reference for this integration — transport, auth, the callback's signature-verification traps, and the subscription-status table. `LIVE_DETECTION.md` Phase 1 covers why it moved off websocket transport; `TWITCH_EVENTSUB.md`'s appendix keeps the lessons from that era.

## Auth

`src/lib/require-api-token.ts` gates `/api/*` and `/sync/*` on `Authorization: Bearer <API_TOKEN>`, compared via sha256 + `timingSafeEqual` (hashed first because `timingSafeEqual` throws on length mismatch, which would leak the token's length).

`/eventsub/*` is deliberately **not** gated — Twitch calls it directly and cannot carry our bearer token. Its safety rests on HMAC signature verification instead (see EventSub section above), which is strictly better than the old `/auth/*` exemption's reliance on "useless without a Twitch-issued code."

`assertApiTokenConfigured()` fails closed: `NODE_ENV=production` (set in the Dockerfile) with no `API_TOKEN` exits rather than quietly serving an open API. Unset outside production just warns.

## Source Resolution

When a VTuber is registered (`POST /api/vtubers`):
- `platform === 'youtube'`: try HoloDex first → `source = 'holodex'`; fall back to YouTube Data API → `source = 'youtube_api'`
- `platform === 'twitch'`: always `source = 'twitch_api'`; `platformChannelId` is stored as the **numeric Twitch user ID**, not the login name

During sync, `VTuber.source` routes to the correct sync function.

## Key Constraints

- **Mapper-at-boundary:** Never store raw external API response shapes. All sync writes go through explicit field mappings inside each sync function. Upstream schema drift should only break one sync function.
- **YouTube quota:** 10,000 units/day. Use direct ID lookups only — never `search.list` (100 units/call). Direct channel/video/playlist lookups are cheap.
- **Upsert pattern:** Streams and Clips use `findOneAndUpdate` with `{ upsert: true, returnDocument: 'after' }` — sync is idempotent.
- **Twitch clips vs YouTube clips:** Conceptually different artifacts (native Twitch clip vs. YouTube community re-upload). Mappers must not assume structural symmetry.
- **Route order matters:** Hono matches top-to-bottom and `:id` is a wildcard, so literal paths must be registered before parameterised siblings.

## Environment Variables

In `.env` (not committed). Note the file uses `KEY = VALUE` with spaces — Bun parses that fine, but Docker's `--env-file` does **not**, which is why the container mounts `.env` rather than passing it.

```
MONGODB_URI=...              # required
HOLODEX_API_KEY=...          # YouTube VTubers on HoloDex
YOUTUBE_API_KEY=...          # YouTube VTubers not on HoloDex, and stats
TWITCH_CLIENT_ID=...         # Twitch VTubers + EventSub
TWITCH_CLIENT_SECRET=...
EVENTSUB_SECRET=...          # signs/verifies EventSub webhook callbacks; openssl rand -base64 32
PUBLIC_URL=...                # https://..., port 443 -- EventSub webhook callback base URL
YOUTUBE_POLL_INTERVAL_MS=... # optional, defaults to 5 minutes
YOUTUBE_POLL_DISABLED=true   # set on the laptop -- local dev shouldn't double-poll the Atlas DB the VPS already polls
STATS_SYNC_INTERVAL_MS=...   # optional, defaults to 6 hours (stats sync poller)
STATS_SYNC_DISABLED=true     # set on the laptop, same reasoning as YOUTUBE_POLL_DISABLED
API_TOKEN=...                # enables bearer auth; mandatory in production
PORT=3000                    # optional, defaults to 3000
NODE_ENV=production          # set by the Dockerfile; makes API_TOKEN mandatory
```

`EVENTSUB_SECRET` and `PUBLIC_URL` must be set on **both** the laptop and the
VPS, to the *same* value — subscriptions are keyed to a callback URL now, not
a session, so both environments end up managing the same shared subscription
set (see EventSub section below).

`YOUTUBE_POLL_DISABLED` is the opposite: deliberately **laptop-only**. Unlike
EventSub subscriptions, the YouTube poller has no shared/callback-bound state
to converge on — if both the laptop and the VPS ran it, they'd redundantly
poll and double-spend YouTube quota against the same Atlas data.

## Scheduled polling

`src/lib/scheduler.ts` runs `startYoutubeLivePoller()`, a self-rescheduling
(not bare `setInterval`) loop that polls for YouTube live status and applies
the same edge-detection `markLive`/`markEnded` (`src/lib/live-state.ts`) that
Twitch's EventSub uses. `YOUTUBE_LIVE.md` at the repo root is the reference for
this half (quota math, the RSS-is-edge-detection-only rule, the failed-poll
guard table); `LIVE_DETECTION.md` Phase 2 is the design it came from. Briefly:
briefly, `source: 'holodex'` VTubers are covered by one unfiltered
`GET /api/v2/live?status=live` call per cycle; `source: 'youtube_api'`
VTubers are covered by each channel's zero-quota RSS feed (edge detection
only -- never trusted to confirm "still live") batched through one
`videos.list` call. Both halves skip their "went offline" pass on a cycle
where the poll itself failed, so a transient outage can't mark everyone
offline. Like `twitch-eventsub.ts` before it, state lives in
`import.meta.hot.data` guarded by `hot.prune()` so `bun --hot` doesn't stack
a second poll loop on every save.

This is the first thing in the codebase that runs sync-adjacent work on a
timer rather than on request -- it operates entirely through `live-state.ts`
and never touches `VTuber.lastLiveSyncedAt`, so it doesn't interact with (or
get gated by) `sync.ts`'s existing staleness checks.

`scheduler.ts` also runs `startStatsSyncPoller()` (added for the TUI
dashboard's trend chart): the three unforced `syncFrom*` calls on a 6h tick.
Unlike the live poller it goes *through* `sync.ts`, deliberately — the 24h
`lastStatsSyncedAt` gate is what makes a 6h tick safe (a cycle where
everyone is fresh costs three DB reads and zero external API calls; the
short tick just bounds worst-case staleness after container restarts).
Before it existed, StatSnapshots were only created opportunistically
(registration, manual sync, Twitch `stream.online`), so most channels had
exactly one snapshot forever and no trend line was possible.
`STATS_SYNC_DISABLED=true` is the laptop-side kill switch, same reasoning
as `YOUTUBE_POLL_DISABLED`.

## Out of Scope

No Redis caching layer, no collab graph, no multi-user accounts (auth is a single shared secret by design). Aggregation dashboard pipelines are planned but not yet implemented.
