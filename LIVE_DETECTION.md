# Live detection — design and implementation plan

Covers three pieces of work that look separate but share a spine:

| Phase | What | Why now |
|---|---|---|
| **0** | Shared live-state writer (`live-state.ts`) | Both phases below need it; also fixes the duplicate-live-docs bug |
| **1** | Twitch: websocket → **webhook** transport | Live tracking is capped at 5 VTubers today and failing silently |
| **2** | YouTube: **polling** + edge detection | No live detection at all for YouTube VTubers |

Phase 0 is a prerequisite for both. Phases 1 and 2 are independent of each
other and can be done in either order.

> **`TWITCH_EVENTSUB.md` has since been rewritten around Phase 1's outcome**
> and now documents the webhook integration as it actually runs. Read it for
> how Twitch EventSub works *here*; read this doc for why it changed. Its
> appendix keeps the websocket-era lessons (the user-token requirement, the
> `4003 "connection unused"` trap) for anyone tempted to go back.

---

# Phase 0 — one writer for live state

## The problem

"This VTuber went live" is currently written in one place
(`twitch-eventsub.ts:handleNotification`), and "went offline" in another shape
in the same function. Phase 2 needs to perform both from a completely
different trigger (a poll loop). Copying that logic gives two divergent
implementations of the most correctness-sensitive write in the system.

There is also an open bug (`bug-report.md` #4): a VTuber can accumulate
multiple `status: 'live'` Stream docs, because `stream.online` upserts a new
live doc without closing any existing one. `/api/vtubers/live` works around
this by deduping. That workaround is load-bearing today.

## What to build

New file: `backend/src/lib/live-state.ts`. Two exported functions, and they
become the **only** places in the codebase that write `status: 'live'` or
transition a stream to `'ended'`.

```ts
/**
 * Mark a VTuber as live on a specific stream.
 *
 * Closes any *other* stream currently marked live for this VTuber before
 * upserting, so a VTuber can never have two live docs. Idempotent: calling
 * it repeatedly for the same externalId is a no-op beyond field refresh.
 *
 * Returns true only on a live -> not-live *edge* (i.e. this VTuber was not
 * already live on this stream), so callers can decide whether to notify.
 */
export async function markLive(
  vtuberId: mongoose.Types.ObjectId,
  stream: IStreamInput
): Promise<boolean>;

/**
 * Mark a VTuber as no longer live.
 *
 * `externalId` optional: Twitch's stream.offline doesn't name the stream, so
 * omitting it means "whatever is currently live for this VTuber". Stamps
 * endTime at call time when the source gives no timestamp.
 *
 * Returns true only if something was actually transitioned.
 */
export async function markEnded(
  vtuberId: mongoose.Types.ObjectId,
  externalId?: string
): Promise<boolean>;
```

`markLive` must, in order:

1. `Stream.updateMany({ vtuberId, status: 'live', externalId: { $ne: stream.externalId } }, { status: 'ended', endTime: new Date() })`
2. `Stream.findOneAndUpdate({ platform, externalId }, stream, { upsert: true, returnDocument: 'after' })`

Step 1 before step 2, so a crash between them leaves zero live docs rather
than two. Zero is self-healing on the next event or poll; two is the bug.

Use the existing `{ platform, externalId }` key — there's already a unique
compound index on it (`Stream.ts`), so the upsert can't race into duplicates.

## Wire it up

- `twitch-eventsub.ts:handleNotification` — replace the inline
  `Stream.findOneAndUpdate` for offline with `markEnded(vtuber._id)`. The
  online branch calls `syncFromTwitch(id, true)`; leave that, but see below.
- `sync.ts` — the three sync functions upsert streams directly. Route the
  ones that write `status: 'live'` through `markLive`. Non-live upserts
  (VODs, ended streams, `upcoming`) can keep using `findOneAndUpdate`.

## Verify

- Manually insert two `status:'live'` docs for one VTuber, call `markLive`
  for a third, confirm exactly one live doc remains.
- Then **remove the dedup workaround** in `GET /api/vtubers/live`
  (`vtubers.ts`) and confirm `oshihub live` still shows each VTuber once.
  Removing it is the acceptance test — if it can't be removed, Phase 0 isn't
  done.
- Close out the stale live docs already in Atlas (there is at least one from
  Tawffie, ~6 days stuck). A one-off script is fine.

---

# Phase 1 — Twitch webhook transport

## Why

Websocket transport has a `max_total_cost` of **10** per (client ID, user ID).
`stream.online` and `stream.offline` each cost **1** for a broadcaster who
hasn't authorized our app — which is every VTuber we track. 2 per VTuber
means a hard ceiling of **5 Twitch VTubers**. We track 6. The 6th fails with
`429 websocket transport subscriptions total cost exceeded` and is silently
never tracked (`bug-report.md` #9).

Webhook transport's `max_total_cost` is **10,000**. The ceiling disappears.

## What it deletes

`getValidUserToken` is imported by exactly one file (`twitch-eventsub.ts`).
Webhooks authenticate subscription management with the **app** token, which
`sync.ts` already uses via `getValidTwitchToken`. So this phase removes:

- `src/lib/twitch-user-token.ts`
- `src/routes/auth.ts` and its mount in `index.ts`
- the `TwitchAuth` model (check `src/models/index.ts` exports first)
- the `/auth/*` auth-exemption comment block in `index.ts`

**Verify each is genuinely unreferenced before deleting** — `grep -rn` for
each symbol. Don't delete on the strength of this doc alone.

Net effect on the security story: the one route that *couldn't* sit behind
the bearer token goes away. The new callback route also can't (Twitch calls
it), but it authenticates every request by HMAC signature, which is strictly
better than "useless without a valid Twitch-issued code."

## The callback route

New file: `backend/src/routes/eventsub.ts`, mounted in `index.ts`.

```
POST /eventsub/callback
```

Must be **exempt from `requireApiToken`** — Twitch cannot send our bearer
token. Because `app.use('/api/*', ...)` and `app.use('/sync/*', ...)` are
path-scoped, mounting at `/eventsub/*` is exempt automatically. Do **not**
mount it under `/api/`.

Twitch sends three kinds of request, distinguished by the
`Twitch-Eventsub-Message-Type` header:

| Header value | Respond with |
|---|---|
| `webhook_callback_verification` | the raw `challenge` string from the body, `200`, `text/plain` |
| `notification` | `204`, then handle the event |
| `revocation` | `204`, and log loudly — subscription is gone |

### Signature verification — read this carefully

This is the part that goes wrong. Compute:

```
message   = Twitch-Eventsub-Message-Id
          + Twitch-Eventsub-Message-Timestamp
          + <raw request body>
expected  = 'sha256=' + HMAC_SHA256(EVENTSUB_SECRET, message).hex()
```

and compare against `Twitch-Eventsub-Message-Signature`.

Three traps:

1. **The raw body, byte for byte.** `JSON.parse` then `JSON.stringify` will
   *not* reproduce it and the HMAC will never match. In Hono, read
   `const raw = await c.req.text()` **first**, verify, then `JSON.parse(raw)`
   yourself. Never call `c.req.json()` before verifying.
2. **Use `crypto.timingSafeEqual`**, and hash both sides to equal length
   first — the same pattern `require-api-token.ts` already uses, and for the
   same reason (`timingSafeEqual` throws on length mismatch).
3. **Reject stale timestamps.** Twitch recommends rejecting anything older
   than **10 minutes** to blunt replay. Also keep a small in-memory
   `Set` of recent message IDs and drop duplicates — Twitch retries on
   non-2xx and will legitimately resend.

### Respond fast

Twitch expects a 2xx **within 10 seconds** and retries otherwise. The
`stream.online` handler currently calls `syncFromTwitch(id, true)`, which is
exactly the multi-second fan-out that caused `bug-report.md` #10. So:
**return 204 first, then do the work fire-and-forget.** Do not await the sync
inside the request handler.

### Secret

New env var `EVENTSUB_SECRET`, 10–100 characters, same value for every
subscription. Generate with `openssl rand -base64 32`. Add to `.env` on both
the laptop and the VPS, and document it in `backend/CLAUDE.md`'s env table.
The backend should refuse to create subscriptions without it.

## Subscription management

`reconcileSubscriptions()` survives mostly intact — same diff-tracked-against-
existing logic. Changes:

- `transport` becomes
  `{ method: 'webhook', callback: `${PUBLIC_URL}/eventsub/callback`, secret }`
- no `session_id`, so `subscribeToLive()` loses its "session not ready yet"
  guard entirely
- new env var `PUBLIC_URL` (e.g. `https://oshihub.195.201.149.64.sslip.io`).
  Must be HTTPS and port 443 — Twitch rejects anything else.
- **status handling changes.** Websocket subs go `websocket_disconnected`;
  webhook subs go `webhook_callback_verification_pending` (transient — do NOT
  delete these, they're mid-handshake), `webhook_callback_verification_failed`
  and `notification_failures_exceeded` (both terminal — delete and recreate).
  The current code deletes anything `!== 'enabled'`, which would
  thrash the pending state. This needs fixing, not copying.
- **Subscriptions now survive restarts.** They're bound to a URL, not a
  session. Reconcile on boot is still right (self-heals drift), but it stops
  being load-bearing.

## What gets simpler operationally

The "only one backend may run at a time" constraint largely dissolves.
Subscriptions point at the production callback URL, so a local backend simply
won't receive events — instead of two backends fighting over websocket
subscription coverage and Twitch closing one with `4003`. Update
`CLAUDE.md` and `OPS.md` when this lands.

## Local development

Local can't receive webhooks. Use the Twitch CLI:

```sh
twitch event trigger stream.online \
  -F http://localhost:3000/eventsub/callback \
  -s "$EVENTSUB_SECRET"
```

It signs the payload correctly, so this exercises the real verification path.
Also test `webhook_callback_verification` with `twitch event verify-subscription`.

## Verify

1. Local: Twitch CLI trigger → 204, correct DB write.
2. Local: tamper one byte of the signature → rejected 403.
3. Deploy, then confirm all subscriptions reach `status: 'enabled'` —
   `GET /helix/eventsub/subscriptions` with the app token.
4. **Confirm `total_cost` is now well under `max_total_cost: 10000`,
   and that all 6 VTubers have both event types.** This is the whole point
   of the phase; check it explicitly rather than assuming.
5. Real go-live: watch `docker logs -f oshihub` for a genuine `stream.online`.

---

# Phase 2 — YouTube live detection by polling

> **Implemented — see [`YOUTUBE_LIVE.md`](YOUTUBE_LIVE.md)** for how the result
> actually behaves. This section is the plan and the API research it rests on.

YouTube has no usable push. Research below was verified against the live APIs
on 2026-07-25.

> **WebSub/PubSubHubbub is still out** — but not for the reason previously
> recorded. The old note said it needed a public HTTPS callback we lacked;
> after Phase 1 we have exactly that. It's ruled out because it fires on
> video **publish**, not on going **live** — a stream scheduled hours ahead
> notifies at schedule time. Wrong signal, not wrong infrastructure.

## Two populations, two methods

**`source: 'holodex'` → one unfiltered poll.** `GET /api/v2/live?status=live`
with no channel filter returns *all* globally-live streams (134 entries /
92KB when measured), each with `channel.id`. Filter client-side against
tracked IDs. One request per cycle regardless of VTuber count.
`channel_id` does **not** accept comma-separated lists — tested, returns 0
results — so per-channel requests are strictly worse.

**`source: 'youtube_api'` → free RSS, then batched `videos.list`.** These are
by definition the channels *not* in Holodex, so the poll above misses them.

- `https://www.youtube.com/feeds/videos.xml?channel_id=UC...` — **zero quota,
  no API key**, 15 most recent video IDs.
- Verified newly-live streams do appear (5/5 found, 4–10 min after going
  live). **They do not appear once they scroll off the 15-entry window** — an
  earlier test wrongly concluded RSS was unreliable because it used a
  24-day-old stream. So RSS is valid for *edge detection only*, never
  backfill.
- Batch the collected IDs into
  `videos.list?part=snippet,liveStreamingDetails&id=<up to 50>` — **1 unit
  per 50 IDs** — and treat `snippet.liveBroadcastContent === 'live'` as live.

Quota: ~1 unit per cycle for ~10 VTubers ⇒ **288 units/day at 5-minute
polling**, against a 10,000/day budget. The naive per-VTuber path costs
2 units each and would be 5,760/day.

## Edge detection

EventSub delivers discrete events; polling delivers state snapshots. Convert:

- in poll result, not `status:'live'` in Mongo ⇒ **went live** → `markLive()`
- `status:'live'` in Mongo, absent from poll ⇒ **went offline** → `markEnded()`

Scope the "absent from poll" side to the VTubers that poll actually covers —
a Holodex outage must not mark every YouTube VTuber offline. Guard: if a poll
returns zero results where the previous cycle had many, treat it as a failed
poll and skip the offline pass entirely.

## The missing infrastructure: there is no scheduler

Grepped — no `setInterval`, no cron, anywhere in the backend. This phase has
to introduce one.

New file: `backend/src/lib/scheduler.ts`, started from `index.ts`.

**It must use the `import.meta.hot.data` guard that `twitch-eventsub.ts`
uses**, or `bun --hot` stacks a fresh interval on every file save and the
poll rate silently multiplies as you work. Copy that pattern (`hot.prune`,
not `hot.dispose` — see the comment in `twitch-eventsub.ts:31`).

Also: don't use a bare `setInterval` around an async function, or a slow
cycle overlaps the next one. Use a self-rescheduling loop that starts the
next timer after the current cycle settles.

Default 5-minute interval, `YOUTUBE_POLL_INTERVAL_MS` to override, and a way
to disable it entirely so local dev doesn't double-poll against the same
Atlas database the VPS is polling.

Once this exists, the 15-minute staleness gate in `sync.ts` finally does
something automatic.

## Verify

- Point the poller at a VTuber known to be live; confirm one `markLive`.
- Confirm a second cycle produces **no** duplicate and no repeated log line —
  edge detection, not level detection.
- Kill the network mid-cycle; confirm nothing gets marked offline.
- Watch quota in the Google Cloud console over 24h; expect a few hundred
  units, not thousands.

---

# Suggested order

1. **Phase 0** — small, self-contained, and its acceptance test (deleting the
   `/api/vtubers/live` dedup) proves bug #4 is genuinely fixed.
2. **Phase 1** — highest urgency: a tracked VTuber is invisible right now.
3. **Phase 2** — largest, and benefits from Phase 0 being settled first.

Phase 1 and 2 both end with "and now `oshihub live` is actually correct",
which is the real goal.
