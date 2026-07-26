// YouTube has no push equivalent to Twitch's EventSub, so live detection here
// is polling + edge detection instead of an event trigger. See
// LIVE_DETECTION.md (Phase 2) for the full design and quota math.
//
// Two tracked populations need two different mechanisms:
//
// - `source: 'holodex'` -- one unfiltered `GET /api/v2/live?status=live` call
//   returns *every* globally-live Holodex-tracked channel; filter client-side
//   against our tracked IDs. One request per cycle regardless of VTuber count.
// - `source: 'youtube_api'` -- these are by definition channels Holodex
//   doesn't cover. Each channel's zero-quota RSS feed
//   (feeds/videos.xml?channel_id=...) surfaces a newly-live stream within a
//   few minutes, but ONLY as an edge signal: a stream scrolls off the feed's
//   15-entry window well before it ends, so RSS must never be trusted to
//   confirm "still live" or "went offline" on its own. Collected IDs (RSS
//   candidates + whatever we currently believe is live) are batched into one
//   `videos.list` call -- 1 quota unit per 50 IDs -- which is the only source
//   of truth for actual live status.
//
// Both populations feed the same edge-detection rule EventSub gets for free
// from discrete events: in this cycle's poll, not `status:'live'` in Mongo =>
// went live => markLive(); `status:'live'` in Mongo, not confirmed live by
// this cycle's poll => went offline => markEnded().

import { VTuber, Stream } from '../models';
import { markLive, markEnded } from './live-state';
import { mapHolodexStream } from './mappers/holodex.mappers';
import { mapYoutubeStream } from './mappers/youtube.mapper';

const DEFAULT_INTERVAL_MS = 5 * 60 * 1000;
const YOUTUBE_VIDEOS_BATCH_SIZE = 50;
// Per channel, per cycle: only the newest few RSS entries are ever relevant --
// a stream that just went live will be at or near the top. Keeping this small
// is what keeps total candidate IDs (and therefore YouTube quota) low.
const RSS_CANDIDATES_PER_CHANNEL = 3;

interface SchedulerState {
  timer: ReturnType<typeof setTimeout> | null;
  // Previous cycle's *unfiltered* Holodex live count, to distinguish "a real
  // quiet cycle" from "Holodex is down and returned nothing" -- see
  // pollHolodexPopulation().
  previousHolodexLiveTotal: number;
}

// `import.meta.hot` only exists under `bun --hot`. Stashing state in
// hot.data means a file edit reuses it instead of starting a second,
// overlapping poll loop; hot.prune() (not dispose()) only fires when this
// module is fully removed from the graph, not on every ordinary edit --
// same reasoning as the pattern this replaced in twitch-eventsub.ts.
const hot = (import.meta as any).hot;
const state: SchedulerState = hot
  ? (hot.data.youtubeScheduler ??= { timer: null, previousHolodexLiveTotal: 0 })
  : { timer: null, previousHolodexLiveTotal: 0 };

if (hot) {
  hot.prune(() => {
    if (state.timer) clearTimeout(state.timer);
    state.timer = null;
  });
}

async function pollHolodexPopulation(): Promise<void> {
  const apiKey = process.env.HOLODEX_API_KEY;
  if (!apiKey) return;

  const tracked = await VTuber.find({ platform: 'youtube', source: 'holodex', isTracked: true });
  if (tracked.length === 0) return;
  const trackedByChannelId = new Map(tracked.map((v) => [v.platformChannelId, v]));

  let items: any[];
  try {
    const res = await fetch('https://holodex.net/api/v2/live?status=live', {
      headers: { 'X-APIKEY': apiKey },
    });
    if (!res.ok) {
      console.error(`Holodex live poll failed: ${res.status} ${await res.text()}`);
      return;
    }
    items = await res.json();
  } catch (err) {
    console.error('Holodex live poll request failed:', err);
    return;
  }

  // A global outage returning 0 where we previously saw many looks nothing
  // like "everyone we track happened to go offline" -- treat it as a failed
  // poll and skip the offline pass for this cycle only. A cycle or two later,
  // if it's genuinely 0 again, previousHolodexLiveTotal is 0 too and the
  // offline pass resumes normally.
  const skipOfflinePass = items.length === 0 && state.previousHolodexLiveTotal > 0;
  state.previousHolodexLiveTotal = items.length;

  const liveVtuberIds = new Set<string>();
  for (const item of items) {
    const vtuber = trackedByChannelId.get(item.channel?.id);
    if (!vtuber) continue;

    const mapped = mapHolodexStream(item, vtuber._id.toString());
    if (mapped.status !== 'live') continue;

    liveVtuberIds.add(vtuber._id.toString());
    const wentLive = await markLive(vtuber._id, mapped);
    if (wentLive) console.log(`${vtuber.name} is now live on YouTube (Holodex poll)`);
  }

  if (skipOfflinePass) {
    console.warn(
      `Holodex live poll returned 0 results (previous cycle had ${state.previousHolodexLiveTotal}) -- treating as a failed poll, skipping offline pass`
    );
    return;
  }

  for (const vtuber of tracked) {
    if (liveVtuberIds.has(vtuber._id.toString())) continue;
    const ended = await markEnded(vtuber._id);
    if (ended) console.log(`${vtuber.name} is no longer live on YouTube (Holodex poll)`);
  }
}

function extractRecentVideoIds(rssXml: string, limit: number): string[] {
  const ids: string[] = [];
  for (const match of rssXml.matchAll(/<yt:videoId>([\w-]+)<\/yt:videoId>/g)) {
    ids.push(match[1]);
    if (ids.length >= limit) break;
  }
  return ids;
}

async function pollYoutubeApiPopulation(): Promise<void> {
  const apiKey = process.env.YOUTUBE_API_KEY;
  if (!apiKey) return;

  const tracked = await VTuber.find({ platform: 'youtube', source: 'youtube_api', isTracked: true });
  if (tracked.length === 0) return;
  const byChannelId = new Map(tracked.map((v) => [v.platformChannelId, v]));

  const candidateIds = new Set<string>();
  for (const vtuber of tracked) {
    try {
      const res = await fetch(`https://www.youtube.com/feeds/videos.xml?channel_id=${vtuber.platformChannelId}`);
      if (!res.ok) continue;
      const xml = await res.text();
      for (const id of extractRecentVideoIds(xml, RSS_CANDIDATES_PER_CHANNEL)) candidateIds.add(id);
    } catch (err) {
      console.error(`RSS fetch failed for ${vtuber.name}:`, err);
    }
  }

  // RSS is edge-detection only -- never trust "not in this cycle's RSS" to
  // mean "went offline". Directly re-check anything we currently believe is
  // live, regardless of whether it's still in the RSS window.
  const currentlyLive = await Stream.find({
    platform: 'youtube',
    sourceApi: 'youtube_api',
    status: 'live',
    vtuberId: { $in: tracked.map((v) => v._id) },
  });
  for (const s of currentlyLive) candidateIds.add(s.externalId);

  if (candidateIds.size === 0) return;

  const idsArray = [...candidateIds];
  const batches: string[][] = [];
  for (let i = 0; i < idsArray.length; i += YOUTUBE_VIDEOS_BATCH_SIZE) {
    batches.push(idsArray.slice(i, i + YOUTUBE_VIDEOS_BATCH_SIZE));
  }

  const liveVideoIds = new Set<string>();
  let anyBatchFailed = false;

  for (const batch of batches) {
    try {
      const url = `https://www.googleapis.com/youtube/v3/videos?part=snippet,liveStreamingDetails,contentDetails&id=${batch.join(',')}&key=${apiKey}`;
      const res = await fetch(url);
      if (!res.ok) {
        console.error(`YouTube videos.list batch failed: ${res.status}`);
        anyBatchFailed = true;
        continue;
      }
      const data = (await res.json()) as any;
      for (const video of data.items ?? []) {
        const vtuber = byChannelId.get(video.snippet?.channelId);
        if (!vtuber) continue;

        const mapped = mapYoutubeStream(video, vtuber._id.toString());
        if (mapped.status !== 'live') continue;

        liveVideoIds.add(video.id);
        const wentLive = await markLive(vtuber._id, mapped);
        if (wentLive) console.log(`${vtuber.name} is now live on YouTube (RSS/videos.list poll)`);
      }
    } catch (err) {
      console.error('YouTube videos.list batch request failed:', err);
      anyBatchFailed = true;
    }
  }

  if (anyBatchFailed) {
    // A currently-live stream might have been in the failed batch -- can't
    // trust "not seen live this cycle" to mean "went offline" when we didn't
    // actually get a fresh answer for every ID we asked about.
    console.warn('A videos.list batch failed this cycle -- skipping offline pass to avoid a false "went offline"');
    return;
  }

  // Anything we thought was live whose ID either came back non-live or
  // wasn't returned at all (deleted/privated) has genuinely ended.
  const trackedById = new Map(tracked.map((v) => [v._id.toString(), v]));
  for (const s of currentlyLive) {
    if (liveVideoIds.has(s.externalId)) continue;
    const ended = await markEnded(s.vtuberId, s.externalId);
    if (ended) {
      const name = trackedById.get(s.vtuberId.toString())?.name ?? s.title;
      console.log(`${name} is no longer live on YouTube (RSS/videos.list poll)`);
    }
  }
}

export async function pollCycle(): Promise<void> {
  await pollHolodexPopulation().catch((err) => console.error('Holodex live poll cycle failed:', err));
  await pollYoutubeApiPopulation().catch((err) => console.error('YouTube API live poll cycle failed:', err));
}

function scheduleNext(intervalMs: number): void {
  state.timer = setTimeout(() => {
    pollCycle()
      .catch((err) => console.error('YouTube live poll cycle failed:', err))
      .finally(() => scheduleNext(intervalMs));
  }, intervalMs);
}

/**
 * Starts the YouTube live-detection poll loop. Not a bare `setInterval`: each
 * cycle schedules the next timer only after it settles, so a slow cycle
 * (network hiccup, many tracked VTubers) can't overlap the next one.
 */
export function startYoutubeLivePoller(): void {
  if (process.env.YOUTUBE_POLL_DISABLED === 'true') {
    console.warn('YOUTUBE_POLL_DISABLED=true — skipping YouTube live poller');
    return;
  }
  if (state.timer) {
    // Survived a hot reload via import.meta.hot.data — reuse it.
    return;
  }

  const intervalMs = Number(process.env.YOUTUBE_POLL_INTERVAL_MS) || DEFAULT_INTERVAL_MS;

  pollCycle()
    .catch((err) => console.error('YouTube live poll cycle failed:', err))
    .finally(() => scheduleNext(intervalMs));
}
