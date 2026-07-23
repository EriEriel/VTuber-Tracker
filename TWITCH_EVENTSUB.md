# Twitch EventSub Integration — Plan

Status: **implemented**, including `stream.offline`. This document captures the
approach for adding real-time "went live"/"went offline" notifications for
Twitch-sourced VTubers, so the project doesn't have to poll Twitch on a timer
once a scheduler eventually exists. Sections below are
updated in place where implementation diverged from the original plan (auth
requirements, the reconcile logic, hot-reload behavior) — look for the
"corrected"/"gotcha found" callouts. See the task list at the bottom for exactly
what's confirmed working vs. still unverified.

## Goal

React to a tracked Twitch VTuber going live within seconds, instead of waiting for
the next `POST /api/sync/twitch` call to notice via the existing 15-minute
`lastLiveSyncedAt` staleness gate (see README § Sync behavior).

## Why WebSocket transport, not Webhook

Twitch EventSub supports two transports: **webhook** (Twitch POSTs to a public HTTPS
URL you host) and **websocket** (you open an outbound connection to Twitch and it
pushes events over it). This project should use **websocket**, because:

- The backend runs on `localhost:3000` with no public URL, no TLS, and no plan for
  either (README: "hardcoded, no config yet"). Webhook transport is a non-starter
  without standing up a reverse proxy / tunnel just for this feature.
- Websocket only requires an *outbound* connection, using credentials the backend
  already has — no new attack surface, no inbound port to expose.
- At personal-tracker scale (a handful to a few dozen Twitch VTubers), a single
  websocket connection is nowhere near its subscription ceiling (see Limits below).

## Auth: needs a user access token (corrected from the original plan)

The original version of this section claimed the existing app access token
(`getValidTwitchToken()`, client-credentials grant) would work, on the reasoning
that `stream.online` needs no scopes. That's true for **webhook** transport, but
wrong for **websocket** transport — verified by actually calling Twitch's API
during implementation and getting back `400 "invalid transport and auth
combination"`. The rule is: **webhook transport uses an app token, websocket
transport always uses a user token**, regardless of what the subscribed event
needs. Twitch splits the requirement by transport, not by event.

This meant adding a piece the original plan didn't scope: a one-time interactive
OAuth Authorization Code flow.

- `backend/src/models/TwitchAuth.ts` — singleton document storing the user's
  access token, refresh token, and expiry. Stored in Mongo rather than `.env`,
  consistent with how everything else in this project persists state.
- `backend/src/routes/auth.ts` — `GET /auth/twitch/login` redirects to Twitch's
  consent screen (no scopes requested); `GET /auth/twitch/callback` exchanges the
  returned code for tokens and saves them. Requires
  `http://localhost:3000/auth/twitch/callback` to be registered as a redirect URI
  in the Twitch app's dev console — a manual, one-time step outside this codebase.
- `backend/src/lib/twitch-user-token.ts` — `getValidUserToken()`, mirroring
  `twitch-token.ts`'s cache-and-refresh shape but backed by Mongo instead of an
  in-memory variable, since a refresh token has to survive process restarts.
  Twitch rotates the refresh token on every use, so each refresh must persist the
  *new* refresh token, not just the new access token.

`twitch-eventsub.ts`'s `helixFetch()` uses `getValidUserToken()` for all
subscription management calls (create/list/delete). The app token in
`twitch-token.ts` is untouched and still used for everything else (`sync.ts`'s
Helix calls, `fetchTwitchUserById()`, etc.) — only EventSub's websocket transport
needed the switch.

## Architecture

New module: `backend/src/lib/twitch-eventsub.ts`, sibling to `twitch-token.ts` and
`sync.ts`. Responsibilities:

- Own the single websocket connection's lifecycle (connect, reconnect, teardown).
- Expose `subscribeToLive(broadcasterId: string)` and
  `unsubscribeFromLive(broadcasterId: string)` for other modules to call.
- On `stream.online`, **don't reimplement stream-fetching** — call the existing
  `syncFromTwitch(vtuberId, force=true)` (`sync.ts:379`) for just that one VTuber.
  EventSub's job is only to be the trigger; the already-tested Helix→Mongo mapping
  pipeline (`mapTwitchLiveStream`, `mapTwitchStatSnapshot`, etc.) does the rest.
  This matters because EventSub's `stream.online` payload is intentionally minimal
  (`id`, `broadcaster_user_id/login/name`, `type`, `started_at` — no title, no
  thumbnail, no game), so a real Helix call is required anyway to get displayable
  data.
- On `stream.offline`, handle it directly instead of calling `syncFromTwitch` —
  the event carries even less than `stream.online` (just the broadcaster's
  identity, no timestamp at all), so there's nothing worth a real Helix round
  trip for. Find the VTuber's `live` `Stream` doc and flip it to `status: 'ended'`,
  stamping `endTime` with the notification's arrival time rather than anything
  from the payload.

### Startup flow (`index.ts`)

```
connectToDatabase()
  → startEventSubListener()   // new, awaited or fire-and-forget after DB is up
Bun.serve({ ... })
```

`startEventSubListener()`:

1. Open `new WebSocket('wss://eventsub.wss.twitch.tv/ws')` (Bun has a native
   `WebSocket` global — no extra dependency needed).
2. On `session_welcome`, capture `payload.session.id`.
3. Within the 10-second subscribe deadline (see Limits), query
   `VTuber.find({ platform: 'twitch', isTracked: true })` and, for each,
   `POST https://api.twitch.tv/helix/eventsub/subscriptions` **twice** — once
   for `type: 'stream.online'`, once for `type: 'stream.offline'` — both with
   `condition: { broadcaster_user_id: platformChannelId }`,
   `transport: { method: 'websocket', session_id }`.
4. On `notification`, branch on `subscription.type`: `stream.online` looks up
   the VTuber by `platformChannelId === event.broadcaster_user_id` and calls
   `syncFromTwitch(vtuber._id, true)`; `stream.offline` looks up the same way
   and instead directly updates the matching `live` `Stream` doc to
   `status: 'ended'` + `endTime: new Date()`.
5. On `session_keepalive`: no-op (just proof the connection is alive).
6. On `session_reconnect`: open a new connection to `payload.session.reconnect_url`,
   swap once its `session_welcome` arrives, close the old socket. Twitch retains all
   subscriptions across this handoff — **no resubscription needed**, unlike a fresh
   connect.
7. On `revocation` (e.g. broadcaster's auth revoked, or Twitch invalidates a sub):
   log it; the affected VTuber falls back to normal polling next sync.

### Reconciling on startup

Before creating subscriptions, call `GET /helix/eventsub/subscriptions` and diff
against the current `isTracked` Twitch VTuber list:

- Subscriptions that exist for a VTuber no longer tracked (or deleted) → delete via
  `DELETE /helix/eventsub/subscriptions?id=`.
- Tracked VTubers with no existing subscription → create.

Since both `stream.online` and `stream.offline` are now tracked, coverage is
keyed on the pair `` `${broadcasterId}:${type}` ``, not just the broadcaster —
a VTuber can have one event type covered and be missing the other. This is
what makes the `stream.offline` rollout self-healing: every already-tracked
VTuber has `stream.online` coverage but no `stream.offline` subscription yet,
and reconcile's per-pair diff should create just the missing half on the next
`session_welcome` — not yet confirmed by an actual restart (see task list).

**Gotcha found during implementation, not anticipated by this plan:** a
subscription's `status` field matters, not just whether one exists for a given
broadcaster. When a websocket session closes, Twitch doesn't delete its
subscriptions — it flips their `status` to `websocket_disconnected` (observed
directly; not documented up front). Those dead subscriptions still show up in
`GET /helix/eventsub/subscriptions` and will never deliver another event, since
they're bound to a session_id that no longer exists. The first implementation
treated "a subscription exists for this broadcaster" as "already covered," which
meant a genuinely fresh connection could see a stale disconnected subscription,
conclude no new one was needed, and end up with zero live subscriptions attached
to it. Twitch then closed that connection after its 10-second grace period with
close code `4003 "connection unused"` — a real, reproduced failure, not a
hypothetical. The fix: only subscriptions with `status === 'enabled'` count as
live coverage; everything else gets deleted during reconcile regardless of
whether its broadcaster is still tracked.

This also protects against a real gotcha in **this specific project's dev setup**:
`bun run dev` runs `bun run --hot src/index.ts`. The implementation uses
`import.meta.hot.data` (not a plain module-level `let`) to carry the socket across
reloads, plus `import.meta.hot.prune()` — Bun's own docs specifically warn that
`dispose()` would reopen the websocket on every single save, since it fires on
every hot update rather than only when a module is actually removed from the
graph.

One thing observed empirically that the original plan didn't anticipate: this
survives edits to *other* files fine (e.g. editing `routes/vtubers.ts` while the
connection is up left `twitch-eventsub.ts`'s state untouched), but editing
`twitch-eventsub.ts` **itself** reliably dropped the connection anyway during
implementation — likely because changing this module's own import set (e.g.
swapping which token function it imports) changes the shape of the dependency
graph edge, not just the module body, which Bun's HMR seems to treat differently
than an in-place body edit. Net effect: hot-reloading this specific file is not
fully reliable for preserving the connection, so a real process restart is the
trustworthy way to test changes to it — hot reload is still fine for changes made
*elsewhere* in the app while EventSub is running.

### Dynamic subscribe/unsubscribe hooks

`backend/src/routes/vtubers.ts` gets two new call sites:

- `POST /api/vtubers` (platform `twitch`, after successful registration) →
  `subscribeToLive(platformChannelId)`, so a newly-added streamer doesn't wait for
  a backend restart to get live coverage. Creates both `stream.online` and
  `stream.offline` subscriptions.
- `DELETE /api/vtubers/:id` (platform `twitch`) → `unsubscribeFromLive(...)` in the
  existing cascade-delete block, alongside the `Stream`/`Clip`/`StatSnapshot`
  cleanup that's already there. Deletes both subscriptions for that broadcaster.

## Data model

`syncFromTwitch()` already writes into the existing `Stream` model
(`status: 'live'`, unique on `{platform, externalId}` — `Stream.ts:50`), which is
exactly the shape a "went live" event should produce — no changes needed there.

`stream.offline`'s handler queries the same model directly:
`Stream.findOneAndUpdate({ vtuberId, status: 'live' }, { status: 'ended', endTime:
new Date() })`. No schema changes needed here either — the `{vtuberId, status}`
compound index already on `Stream` (`Stream.ts:52`) exists for filtering by
status and covers this query for free. Looking up by `{vtuberId, status: 'live'}`
rather than by `externalId` is deliberate: the `stream.offline` event doesn't
include `externalId` (or any stream identifier) at all, only the broadcaster —
so "the current live stream for this VTuber" is the only handle available. The
update is naturally idempotent: if the event ever arrived twice, the second
call would just match zero documents.

One new collection *was* needed that the original plan didn't scope: `TwitchAuth`
(`backend/src/models/TwitchAuth.ts`), a singleton document holding the user
access/refresh token pair the auth correction above required.

## Limits (verified against current Twitch docs, 2026-07)

- **300 subscriptions per websocket connection** — no issue at this project's
  scale; would only matter past ~300 tracked Twitch VTubers on one connection.
- **10 seconds** to create the first subscription after `session_welcome`, or
  Twitch closes the connection.
- On a server-initiated **reconnect**, subscriptions are preserved automatically;
  the client just has to follow `reconnect_url` within 30 seconds.

Sources:
- [Handling WebSocket Events](https://dev.twitch.tv/docs/eventsub/handling-websocket-events)
- [WebSocket Messages reference](https://dev.twitch.tv/docs/eventsub/websocket-reference/)

## Non-goals for this pass

- **Webhook transport** — revisit only if the backend ever gets a public URL.
- **YouTube live notifications** — no native "went live" push exists; separate
  design (likely polling HoloDex's aggregated live status, not YouTube directly —
  see prior discussion). Not part of this document.
- **Actual notification delivery beyond the database** (Discord webhook, desktop
  push, etc.) — out of scope for the first pass. The `stream.online`/`stream.offline`
  handlers are a natural place to add a Discord webhook `fetch()` call later, once
  there's somewhere the user actually wants to be pinged; adding it speculatively
  now would be building for a requirement that doesn't exist yet.

## Task list — status

1. [x] `backend/src/lib/twitch-eventsub.ts` — connection lifecycle, message
   handling, `subscribeToLive` / `unsubscribeFromLive`.
2. [x] `backend/src/index.ts` — call `startEventSubListener()` after
   `connectToDatabase()`.
3. [x] `backend/src/routes/vtubers.ts` — wire subscribe on create, unsubscribe on
   delete (Twitch only). Wired but not live-exercised (no VTuber was
   registered/deleted after this landed) — worth a real test next time either
   route is touched.
4. [x] `backend/src/models/TwitchAuth.ts`, `backend/src/lib/twitch-user-token.ts`,
   `backend/src/routes/auth.ts` — the OAuth user-token flow the original plan
   didn't scope (see "Auth" section above).
5. [x] Confirmed via `GET /helix/eventsub/subscriptions` (temporary debug routes,
   removed after use): connection reaches `session_welcome`, reconcile creates a
   live `status: 'enabled'` subscription bound to the current session, and dead
   `websocket_disconnected` subscriptions from earlier test connections get
   cleaned up correctly.
6. [x] Verified end-to-end using the
   [Twitch CLI's event simulator](https://dev.twitch.tv/docs/cli/websocket-event-command):
   `twitch event websocket start-server` (mock EventSub server on
   `127.0.0.1:8080`, which also serves a mock `/eventsub/subscriptions`
   endpoint), temporarily pointed `connect()`'s URL and `helixFetch()`'s base
   URL at it, restarted, then `twitch event trigger stream.online
   --transport=websocket --session=<id> --to-user=118858663`. Tawffie's
   `lastLiveSyncedAt`/`lastStatsSyncedAt`/`lastSyncedAt` all updated within the
   same second as the trigger, confirming `handleNotification()` correctly
   matched the broadcaster and called the real `syncFromTwitch()` (which still
   hit the real Twitch API for the actual stream data — only the EventSub
   plumbing itself was mocked). All temporary URL overrides were reverted
   immediately after; the real connection was re-verified afterward and shows
   the same single `enabled` subscription bound to a real session.
7. [ ] **Not yet verified**: the `session_reconnect` handoff path — never
   naturally triggered during testing, since Twitch only sends it occasionally
   from its own side. (The CLI simulator has a `twitch event websocket
   reconnect` command that could exercise this the same way, if it becomes
   worth testing deliberately.)
8. [x] Implemented `stream.offline` handling: `createSubscription`,
   `listSubscriptions`, `reconcileSubscriptions`, `subscribeToLive`, and
   `unsubscribeFromLive` all generalized from "the one `stream.online`
   subscription per broadcaster" to "one subscription per `(broadcaster,
   EventType)` pair"; `handleNotification` branches on `subscription.type` and
   writes `status: 'ended'` + `endTime: new Date()` directly instead of calling
   `syncFromTwitch`. **Not yet live-tested** — same caveat as item 7 on hot
   reload: this file was edited while the backend may have been running, so a
   real restart plus `twitch event trigger stream.offline
   --transport=websocket --session=<id> --to-user=<id>` (mirroring the
   `stream.online` verification in item 6) is the way to confirm the whole
   path end-to-end, including that reconcile actually backfills the missing
   `stream.offline` subscription for every VTuber that only had `stream.online`
   coverage from before this change.
