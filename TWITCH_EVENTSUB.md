# Twitch EventSub Integration

Status: **implemented and deployed**, over **webhook** transport, covering
`stream.online` and `stream.offline`.

This document describes how the integration works now. The migration that got
it here — websocket → webhook — is specced in [`LIVE_DETECTION.md`](LIVE_DETECTION.md)
Phase 1; the reasoning and the lessons from the original websocket build are
preserved in [Appendix: the websocket era](#appendix-the-websocket-era) at the
bottom, because several of them are still load-bearing knowledge.

## Goal

React to a tracked Twitch VTuber going live within seconds, instead of waiting
for the next `POST /api/sync/twitch` to notice via the 15-minute
`lastLiveSyncedAt` staleness gate.

## Transport: webhook

Twitch offers two transports. This project uses **webhook** — Twitch POSTs to a
public HTTPS URL we host.

The original build used **websocket** (an outbound connection Twitch pushes
over), because there was no public URL. Two things changed that:

1. **The VPS deployment behind Caddy provides a public HTTPS endpoint.**
2. **Websocket transport has a `max_total_cost` of 10 per (client ID, user ID).**
   `stream.online` and `stream.offline` each cost **1** for a broadcaster who
   hasn't authorized our app — which is every VTuber we track. 2 per VTuber
   meant a hard ceiling of **5 Twitch VTubers**, already exceeded in production
   and failing silently with `429 websocket transport subscriptions total cost
   exceeded` (`bug-report.md` #9).

Webhook's `max_total_cost` is **10,000**. The ceiling is gone.

Webhook transport also means subscriptions are bound to a **callback URL, not a
session**, so they survive backend restarts — reconcile-on-boot went from
load-bearing to merely self-healing.

## Auth: the app token

Subscription management (create/list/delete) authenticates with the **app**
access token from `src/lib/twitch-token.ts` — the same one `sync.ts` uses for
every other Helix call. `helixFetch()` in `twitch-eventsub.ts` calls
`getValidTwitchToken()`.

The rule is that **webhook transport uses an app token; websocket transport
requires a user token**, regardless of what the event itself needs — Twitch
splits the requirement by transport, not by event. Because this no longer uses
websocket transport, the entire user-token apparatus was **deleted**:

- ~~`src/lib/twitch-user-token.ts`~~ (`getValidUserToken`)
- ~~`src/models/TwitchAuth.ts`~~ (the singleton token document)
- ~~`src/routes/auth.ts`~~ (the one-time interactive OAuth flow)

There is no interactive authorization step any more, and no redirect URI to
register in the Twitch dev console. Don't reintroduce them.

Incoming callbacks authenticate in the opposite direction, by HMAC signature —
see below.

## Architecture

Two files:

| File | Responsibility |
|---|---|
| `src/lib/twitch-eventsub.ts` | Subscription management + the notification handler. Holds no connection. |
| `src/routes/eventsub.ts` | `POST /eventsub/callback` — signature verification and dispatch. |

`initTwitchEventSub()` is called once from `index.ts` after
`connectToDatabase()`. It doesn't listen for anything — it just kicks off one
`reconcileSubscriptions()` pass and returns. It no-ops with a warning if
`TWITCH_CLIENT_ID`, `EVENTSUB_SECRET`, or `PUBLIC_URL` is unset.

### Event handling

`handleNotification(payload)` branches on `subscription.type`:

- **`stream.online`** → `syncFromTwitch(vtuberId, force=true)`. Deliberately
  *not* reimplemented from the payload, which is intentionally minimal (`id`,
  broadcaster identity, `type`, `started_at` — no title, thumbnail, or game).
  EventSub is only the trigger; the tested Helix→Mongo pipeline does the work.
- **`stream.offline`** → `markEnded(vtuber._id)` from `src/lib/live-state.ts`.
  This event carries even less (no timestamp at all), so there's nothing worth
  a Helix round trip for; `endTime` is stamped on arrival.

Both paths go through `live-state.ts`, which since Phase 0 is the **only** place
in the backend that writes `status: 'live'` or transitions a stream to
`'ended'`. `markLive` closes any other live doc for that VTuber before
upserting, which is what fixed the duplicate-live-docs bug (`bug-report.md` #4)
and let `GET /api/vtubers/live` drop its dedup workaround.

## The callback route

```
POST /eventsub/callback
```

**Exempt from `requireApiToken`** — Twitch cannot send our bearer token.
`index.ts` scopes the middleware with `app.use('/api/*', …)` and
`app.use('/sync/*', …)`, so mounting at `/eventsub/*` is exempt automatically.
**Do not mount it under `/api/`.**

Twitch sends three request kinds, distinguished by `Twitch-Eventsub-Message-Type`:

| Header value | Response |
|---|---|
| `webhook_callback_verification` | the raw `challenge` string, `200`, `text/plain` |
| `notification` | `204`, then handle the event |
| `revocation` | `204`, and log loudly — the subscription is gone |

### Signature verification

```
message  = Twitch-Eventsub-Message-Id
         + Twitch-Eventsub-Message-Timestamp
         + <raw request body>
expected = 'sha256=' + HMAC_SHA256(EVENTSUB_SECRET, message).hex()
```

compared against `Twitch-Eventsub-Message-Signature`. Three things that are
easy to get wrong, all handled in `routes/eventsub.ts`:

1. **The raw body, byte for byte.** `JSON.parse` → `JSON.stringify` will not
   reproduce it and the HMAC will never match. The handler reads
   `await c.req.text()` **first**, verifies, and only then parses. Never call
   `c.req.json()` before verifying.
2. **`crypto.timingSafeEqual`, with both sides hashed to equal length first** —
   the same pattern as `require-api-token.ts`, and for the same reason: it
   throws on a length mismatch.
3. **Stale timestamps are rejected** (older than 10 minutes), and recent message
   IDs are held in an in-memory `Map` so retries are dropped. Twitch retries on
   any non-2xx and will legitimately resend.

### Respond fast

Twitch expects a 2xx **within 10 seconds** and retries otherwise. The
`stream.online` path calls `syncFromTwitch`, a multi-second fan-out (the cause
of `bug-report.md` #10). So the handler **returns 204 first and does the work
fire-and-forget** — the sync is never awaited inside the request.

## Subscription management

`reconcileSubscriptions()` diffs Twitch's actual subscription list against
tracked Twitch VTubers, keyed on `(broadcasterId, type)` pairs — a VTuber can
have one event type covered and be missing the other.

Transport is
`{ method: 'webhook', callback: \`${PUBLIC_URL}/eventsub/callback\`, secret }`.

**Status handling is the subtle part**, and differs from the websocket version:

| Status | Treatment |
|---|---|
| `enabled` | counts as coverage |
| `webhook_callback_verification_pending` | **counts as coverage** — mid-handshake, about to become `enabled` on its own. Deleting it would thrash a subscription that's fine. |
| `webhook_callback_verification_failed` | terminal → delete and recreate |
| `notification_failures_exceeded` | terminal → delete and recreate |
| `websocket_disconnected` | terminal → delete. This codebase no longer creates websocket subscriptions at all, so it can only be a leftover from before the migration. |

The old code deleted anything `!== 'enabled'`, which under webhook transport
would have raced the pending handshake on every boot.

### Dynamic subscribe/unsubscribe

`src/routes/vtubers.ts`:

- `POST /api/vtubers` (Twitch) → `subscribeToLive(platformChannelId)`,
  fire-and-forget, so a newly-registered streamer gets coverage without a
  restart. Creates both event types. Unlike the websocket version, this has no
  "session not ready yet" guard — there's no session.
- `DELETE /api/vtubers/:id` (Twitch) → `unsubscribeFromLive(...)` in the
  existing cascade-delete block.

## Environment

```
TWITCH_CLIENT_ID=...       # also used by sync.ts
TWITCH_CLIENT_SECRET=...
EVENTSUB_SECRET=...        # 10–100 chars; openssl rand -base64 32
PUBLIC_URL=https://...     # HTTPS on port 443 — Twitch rejects anything else
```

`requirePublicUrl()` validates the scheme and port at subscription-creation
time rather than letting Twitch reject it later. `EVENTSUB_SECRET` must be
identical everywhere the backend runs, since subscriptions are shared.

## Limits

- **`max_total_cost` 10,000** for webhook transport (vs 10 for websocket).
- **10 seconds** to return a 2xx, or Twitch retries.
- Callback must be **HTTPS on port 443**.

Sources: [Handling Webhook Events](https://dev.twitch.tv/docs/eventsub/handling-webhook-events),
[EventSub subscription types](https://dev.twitch.tv/docs/eventsub/eventsub-subscription-types/).

## Local development

A local backend can't receive webhooks — subscriptions point at `PUBLIC_URL`,
which is production. This is fine and is why the old "only one backend may run
at a time" constraint dissolved: a local backend's reconcile just confirms the
same shared subscription set, and never receives the traffic.

To exercise the real verification path locally, use the Twitch CLI, which signs
payloads correctly:

```sh
twitch event trigger stream.online \
  -F http://localhost:3000/eventsub/callback \
  -s "$EVENTSUB_SECRET"

twitch event verify-subscription stream.online \
  -F http://localhost:3000/eventsub/callback \
  -s "$EVENTSUB_SECRET"
```

## Verification status

- [x] Signature verification — valid signature accepted; a single tampered byte
      → 403; stale timestamp → 403; duplicate message-id deduped; missing
      headers → 400; `webhook_callback_verification` echoes the raw challenge as
      `text/plain`.
- [x] Deployed, and all tracked Twitch VTubers reach `status: 'enabled'`.
      Confirmed against `GET /helix/eventsub/subscriptions`: 4 VTubers × 2 event
      types = 8 subscriptions, `total_cost` 8 against `max_total_cost` 10,000 —
      the ceiling that motivated the whole migration is gone.
- [x] Subscriptions survive a backend restart (bound to the URL, not a session).
- [ ] **A real, unsimulated `stream.online` from a live channel** — the webhook
      path has not yet been observed end-to-end in production. Watch
      `ssh hetzner 'docker logs -f oshihub'` next time a tracked VTuber goes
      live. (`oshihub watch` firing a desktop notification is the visible
      downstream signal.)
- [ ] `revocation` handling — implemented, never triggered.

---

## Appendix: the websocket era

Kept because the lessons cost real debugging time and some still apply.

**Why websocket was chosen first.** The backend ran on `localhost:3000` with no
public URL and no plan for one, which makes webhook transport a non-starter.
Websocket needed only an outbound connection and no inbound port. That reasoning
was correct at the time; the VPS deployment invalidated its premise.

**Websocket transport requires a user token** — verified the hard way, by
getting `400 "invalid transport and auth combination"` from an app token. This
is what forced the OAuth Authorization Code flow, the `TwitchAuth` singleton,
and the refresh-token rotation handling (Twitch issues a *new* refresh token on
every use, so each refresh had to persist it). All deleted in Phase 1 — but if
anyone ever reintroduces websocket transport, this is the cost.

**`websocket_disconnected` subscriptions and close code `4003`.** When a
websocket session closed, Twitch didn't delete its subscriptions — it flipped
their `status` to `websocket_disconnected`. Those still appeared in
`GET /helix/eventsub/subscriptions` and would never deliver another event, being
bound to a dead session. The first implementation treated "a subscription exists
for this broadcaster" as "covered", so a fresh connection concluded it needed
none, ended up with zero live subscriptions, and Twitch closed it after the
10-second grace period with `4003 "connection unused"`. A reproduced failure,
not a hypothetical. The fix — only `enabled` counts as coverage — is why the
current code still has an explicit status table, and why
`websocket_disconnected` remains in the terminal-delete set to sweep leftovers.

**Hot reload.** The socket lived in `import.meta.hot.data` with
`import.meta.hot.prune()` — not `dispose()`, which Bun's docs warn fires on
every hot update and would reopen the socket on every save. Editing
*other* files preserved the connection; editing `twitch-eventsub.ts` itself
reliably dropped it anyway, likely because changing its import set changes the
dependency-graph edge rather than just the module body. None of this applies to
the current code, which holds no connection — but the same `hot.data` + `prune`
pattern was carried over to `src/lib/scheduler.ts`, where a stacked duplicate
timer on every save is the equivalent hazard.

**Websocket limits**, for reference: 300 subscriptions per connection, 10
seconds to create the first subscription after `session_welcome`, subscriptions
preserved across a server-initiated `session_reconnect` (follow `reconnect_url`
within 30 seconds). The `session_reconnect` handoff was never verified — Twitch
only sends it occasionally, and `twitch event websocket reconnect` was never
run against it. Moot now.
