# YouTube Live Detection

Status: **implemented and deployed**, by polling on a 5-minute cycle.

The Twitch counterpart is [`TWITCH_EVENTSUB.md`](TWITCH_EVENTSUB.md); the two
are deliberately asymmetric, because YouTube has no usable push. The design
rationale and the API research behind it are in
[`LIVE_DETECTION.md`](LIVE_DETECTION.md) Phase 2 — this document describes what
actually runs.

Everything downstream is shared: both platforms write live state exclusively
through `markLive`/`markEnded` in `src/lib/live-state.ts`, surface through
`GET /api/vtubers/live`, and reach the desktop via `oshihub watch`. A
notification does not know or care which platform produced it, which is the
entire point of unifying them.

## Why polling

YouTube has no equivalent of Twitch's EventSub. The one native push mechanism is
**WebSub/PubSubHubbub** (`pubsubhubbub.appspot.com`, topic
`youtube.com/xml/feeds/videos.xml?channel_id=…`), and it's ruled out — but not
for the reason originally assumed.

The old note said it needed a public HTTPS callback we didn't have. That stopped
being true once the VPS landed (it's exactly what Twitch webhooks now use). The
real reason is that **WebSub fires on video *publish*, not on going *live***: a
stream scheduled hours ahead notifies at schedule time, not at start time. Wrong
signal, not wrong infrastructure. Don't revisit it on the assumption that the
blocker was hosting.

So the backend polls and derives edges itself.

## Two populations, two mechanisms

Tracked YouTube VTubers split by `source`, and the split is not cosmetic — the
two groups are, by definition, disjoint, and neither method can cover the other's
channels.

### `source: 'holodex'` — one unfiltered call

`GET https://holodex.net/api/v2/live?status=live` with **no channel filter**
returns *every* currently-live stream Holodex knows about (134 entries / ~92 KB
when measured), each carrying `channel.id`. Filter client-side against tracked
IDs.

One request per cycle regardless of how many VTubers are tracked. Note that
`channel_id` **does not accept a comma-separated list** — tested, it returns 0
results — so the apparently-cheaper per-channel approach is strictly worse.

### `source: 'youtube_api'` — free RSS, then batched `videos.list`

These are by definition the channels Holodex doesn't carry, so the call above
misses them entirely.

1. **`https://www.youtube.com/feeds/videos.xml?channel_id=UC…`** — **zero quota,
   no API key**, returns the 15 most recent video IDs. The scheduler takes only
   the newest 3 per channel (`RSS_CANDIDATES_PER_CHANNEL`), since a stream that
   just started is at or near the top and pulling more only inflates the batch.
2. **`videos.list?part=snippet,liveStreamingDetails,contentDetails&id=…`** — up
   to 50 IDs for **1 quota unit**. `snippet.liveBroadcastContent === 'live'` is
   the actual answer.

**RSS is an edge-detection signal only, never a source of truth.** Verified:
newly-live streams do appear in the feed (5/5 found, 4–10 minutes after going
live), but they scroll off the 15-entry window well before the stream ends. An
earlier test wrongly concluded RSS was unreliable because it checked a 24-day-old
stream.

That property is why the scheduler doesn't just poll RSS and diff. Every stream
currently believed live is added to the batch **directly, regardless of whether
it still appears in RSS** — otherwise a long stream would silently "end" the
moment it aged out of the feed. Guarding against this is the single most
important detail in the YouTube path.

## Quota

| | Cost |
|---|---|
| Holodex poll | 0 YouTube units (different API) |
| RSS feeds | 0 units, no API key |
| `videos.list` | 1 unit per 50 IDs |

At a 5-minute cycle that's roughly **288 units/day** against a 10,000/day
budget. The naive per-VTuber path (`playlistItems` + `videos`, 2 units each)
would cost ~5,760/day for the same coverage.

Holodex-sourced VTubers cost **zero** YouTube quota, since that population never
touches the YouTube Data API.

## Edge detection

EventSub delivers discrete events; polling delivers state snapshots. The
conversion:

- in this cycle's poll, not `status: 'live'` in Mongo → **went live** → `markLive()`
- `status: 'live'` in Mongo, not confirmed live by this cycle → **went offline** → `markEnded()`

`markLive` returns `true` only on a genuine not-live→live edge, so repeated
cycles over an ongoing stream are silent.

### The failed-poll guards

**A failed check and a negative check are indistinguishable in the data, but
only one of them means "offline."** This is the defining hazard of polling-based
detection, and both halves of the scheduler guard against it separately.

| Situation | Behaviour |
|---|---|
| Holodex request throws or returns non-2xx | Log and return. Nothing is written at all. |
| Holodex returns **0 live streams globally** where the previous cycle had many | Treated as an outage: run the went-live pass, **skip the offline pass** for this cycle only. |
| Any `videos.list` batch fails | Skip the offline pass for the `youtube_api` population — a currently-live stream may have been in the failed batch, so "not seen live" isn't a fresh answer. |

The Holodex zero-result guard self-clears: once `previousHolodexLiveTotal` is
also 0, a genuinely quiet cycle resumes normal offline handling.

The CLI mirrors this same rule one layer up — `oshihub watch` also refuses to
mutate state on a failed poll, and additionally requires two consecutive misses
before considering a stream over.

## The scheduler

`src/lib/scheduler.ts`, started by `startYoutubeLivePoller()` from `index.ts`.
This was the **first scheduled work anywhere in the backend** — before it,
nothing ran on a timer.

Two structural details, both deliberate:

- **Self-rescheduling, not `setInterval`.** Each cycle schedules the next timer
  only after the current one settles, so a slow cycle can't overlap the next.
- **State lives in `import.meta.hot.data`, guarded with `hot.prune()`** (not
  `dispose()`, which fires on every hot update). Without this, `bun --hot`
  stacks a fresh timer on every file save and the effective poll rate silently
  multiplies while you work. The pattern is inherited from the old
  `twitch-eventsub.ts` websocket handling.

It operates entirely through `live-state.ts` and never touches
`VTuber.lastLiveSyncedAt`, so it neither triggers nor is gated by `sync.ts`'s
15-minute staleness check. Full syncs (stats, clips, VOD backfill) remain
request-driven.

## Environment

```
HOLODEX_API_KEY=...               # the holodex population; without it that half no-ops
YOUTUBE_API_KEY=...               # the youtube_api population
YOUTUBE_POLL_INTERVAL_MS=300000   # optional, defaults to 5 minutes
YOUTUBE_POLL_DISABLED=true        # optional; set on any machine that shouldn't poll
```

**Run the poller in exactly one place.** Unlike Twitch EventSub — whose
subscriptions are bound to a shared callback URL, so a second backend converges
harmlessly — two pollers just duplicate work and double-spend YouTube quota
against the same Atlas database. `YOUTUBE_POLL_DISABLED=true` is set on the
laptop; the VPS runs it.

## Verification status

- [x] A VTuber known to be live is picked up and produces exactly one
      `markLive` (tested against a real live Holodex channel via a throwaway
      tracked VTuber).
- [x] A second cycle over the same live stream produces **no** duplicate and no
      repeated log line — edge detection, not level detection.
- [x] A simulated mid-cycle failure (Holodex returning 500) marks nothing
      offline and leaves the existing live doc untouched.
- [ ] **Quota observed over a full 24h** in the Google Cloud console — expected
      a few hundred units, not thousands. Worth checking once, since the
      estimate above is arithmetic rather than measurement.
- [ ] A real `youtube_api`-sourced VTuber going live. Only the Holodex half has
      been exercised against live data; the RSS + `videos.list` path is tested
      but has not yet caught a real go-live, because every tracked YouTube
      VTuber currently resolves to `source: 'holodex'`.

## Known gaps and gotchas

- **Latency floor is the poll interval.** Worst case ~5 minutes for Holodex, and
  for `youtube_api` channels the RSS feed's own 4–10 minute lag stacks on top.
  This is accepted — see `TWITCH_EVENTSUB.md` and the README for why the
  notification system's value is unification rather than speed.
- **A stream that starts and ends entirely between two cycles is never seen.**
  Inherent to polling.
- **`mapHolodexStream` decides what counts as live**, not the poller — the
  scheduler only acts on entries whose mapped `status` is `'live'`. Holodex's
  own `status` field (`upcoming`/`live`/`past`) is the input, so an upstream
  change to that vocabulary would silently stop detection rather than error.
- **The `youtube_api` offline pass keys on `externalId`**, unlike the Holodex
  half which keys on the VTuber. That's because `videos.list` answers per-video,
  so it can say precisely *which* stream ended.
