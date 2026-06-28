# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```sh
bun run dev                          # start server with hot reload on port 3000
bun run src/integration.test.ts      # run manual integration test (hits real APIs + DB)
```

There is no build step — Bun runs TypeScript directly. No test framework is configured; `integration.test.ts` is a standalone script.

## Architecture

A Hono/Bun REST API that aggregates VTuber data from three external APIs into a unified MongoDB schema.

**Entry:** `src/index.ts` mounts two real route groups (`vtubersRoute`, `syncRoute`) and three dev/debug test routes (`holodex.test`, `youtube.test`, `twitch.test`).

**Routes:**
- `src/routes/vtubers.ts` — CRUD for VTuber documents; POST registration resolves the source API automatically
- `src/routes/sync.ts` — triggers sync for `POST /api/sync/{holodex,youtube,twitch,all}` with optional `?id=` and `?force=true`

**Sync layer (`src/lib/sync.ts`):** Three source-specific functions (`syncFromHolodex`, `syncFromYoutube`, `syncFromTwitch`). Each enforces two staleness gates per VTuber before hitting external APIs:
- `lastLiveSyncedAt` — skip if fresher than 15 minutes
- `lastStatsSyncedAt` — skip if fresher than 24 hours
- `?force=true` bypasses both gates

**Models (`src/models/`):** Each model file exports both a Zod schema (for input validation) and a Mongoose model. All four collections share a `sourceApi` discriminator field.

**DB (`src/lib/db.ts`):** Singleton connection with race-condition protection — a single `connectionPromise` prevents concurrent `mongoose.connect()` calls during server startup.

**Twitch token (`src/lib/twitch-token.ts`):** Module-level in-memory cache for the OAuth2 app access token with a 60-second expiry buffer. Shared across all Twitch-sourced VTubers.

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

## Environment Variables

Required in `.env` (not committed):
```
HOLODEX_API_KEY=...
YOUTUBE_API_KEY=...
TWITCH_CLIENT_ID=...
TWITCH_CLIENT_SECRET=...
MONGODB_URI=...
```

## Out of Scope (MVP)

No scheduled/cron sync, no authentication, no Redis caching layer, no live notifications, no collab graph. Aggregation dashboard pipelines are planned but not yet implemented.
