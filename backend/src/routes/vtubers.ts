import { Hono } from 'hono';
import { z } from 'zod';
import { VTuber, Stream, Clip, StatSnapshot } from '../models';
import {
  resolveTwitchUser,
  extractYoutubeHandle,
  resolveYoutubeHandle,
  fetchTwitchUserById,
  syncFromHolodex,
  syncFromYoutube,
  syncFromTwitch,
} from '../lib/sync';
import { subscribeToLive, unsubscribeFromLive } from '../lib/twitch-eventsub';

export const vtubersRoute = new Hono();

// Escape regex metacharacters so user input to `$regex` filters is matched
// literally, not compiled as a pattern (avoids ReDoS via crafted input).
function escapeRegex(str: string): string {
  return str.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

// Schema for adding a new VTuber
const CreateVTuberSchema = z.object({
  platform: z.enum(['youtube', 'twitch']),
  channelId: z.string().min(1), // Youtube Channel ID or Twitch Login Name
});

/**
 * POST /api/vtubers
 * Register a new VTuber with automatic source priority resolution
 */
vtubersRoute.post('/api/vtubers', async (c) => {
  try {
    const body = await c.req.json();
    const parsed = CreateVTuberSchema.safeParse(body);

    if (!parsed.success) {
      return c.json({ error: 'Invalid request body', details: parsed.error.format() }, 400);
    }

    const { platform, channelId } = parsed.data;

    // Check if already exists in DB
    // Note: Twitch is saved by numeric ID, so we'll check that after resolving Twitch ID
    if (platform === 'youtube') {
      const existing = await VTuber.findOne({ platform: 'youtube', platformChannelId: channelId });
      if (existing) {
        return c.json({ error: 'VTuber already registered', vtuber: existing }, 409);
      }
    }

    let name = '';
    let englishName = '';
    let photo = '';
    let source!: 'holodex' | 'youtube_api' | 'twitch_api';
    let platformChannelId = channelId;
    let org = undefined;
    let suborg = undefined;

    if (platform === 'youtube') {
      // Normalize a pasted URL (https://www.youtube.com/@handle) down to the bare @handle.
      // Both HoloDex and the YouTube fallback below need this same normalized form.
      const handle = extractYoutubeHandle(channelId);
      const lookupId = handle || channelId;

      // 1. Try HoloDex first
      const holodexKey = process.env.HOLODEX_API_KEY;
      let holodexSuccess = false;

      if (holodexKey) {
        try {
          const holodexUrl = `https://holodex.net/api/v2/channels/${lookupId}`;
          const res = await fetch(holodexUrl, {
            headers: { 'X-APIKEY': holodexKey }
          });

          if (res.ok) {
            const data = await res.json() as any;
            if (data && data.id) {
              name = data.name || '';
              englishName = data.english_name || name;
              photo = data.photo || '';
              org = data.org;
              suborg = data.suborg;
              source = 'holodex';
              holodexSuccess = true;
              platformChannelId = data.id; // canonical channel ID HoloDex resolved, not the raw input
            }
          }
        } catch (err) {
          console.error('HoloDex lookup failed, falling back to YouTube API:', err);
        }
      }

      // 2. Fall back to YouTube Data API v3
      if (!holodexSuccess) {
        const youtubeKey = process.env.YOUTUBE_API_KEY;
        if (!youtubeKey) {
          return c.json({ error: 'HoloDex lookup failed and YOUTUBE_API_KEY is not set' }, 500);
        }

        if (handle) {
          // Not on HoloDex -- resolve the handle directly via YouTube's API instead of storing it as-is.
          const resolved = await resolveYoutubeHandle(handle);
          if (!resolved) {
            return c.json({ error: `YouTube handle "${handle}" not found` }, 404);
          }
          name = resolved.name;
          englishName = resolved.englishName;
          photo = resolved.photo;
          platformChannelId = resolved.id;
          source = 'youtube_api';
        } else {
          const ytUrl = `https://www.googleapis.com/youtube/v3/channels?part=snippet,statistics&id=${channelId}&key=${youtubeKey}`;
          const res = await fetch(ytUrl);

          if (!res.ok) {
            const errMsg = await res.text();
            return c.json({ error: 'YouTube channel lookup failed', details: errMsg }, 502);
          }

          const data = await res.json() as any;
          const channel = data.items?.[0];

          if (!channel) {
            return c.json({ error: `YouTube channel "${channelId}" not found` }, 404);
          }

          name = channel.snippet?.title || '';
          englishName = name;
          photo = channel.snippet?.thumbnails?.high?.url || channel.snippet?.thumbnails?.default?.url || '';
          source = 'youtube_api';
        }
      }
    } else {
      // platform === 'twitch'
      // Always Twitch Helix API
      source = 'twitch_api';
      try {
        const twitchUser = await resolveTwitchUser(channelId);
        if (!twitchUser) {
          return c.json({ error: `Twitch user "${channelId}" not found` }, 404);
        }

        platformChannelId = twitchUser.id; // numeric Twitch ID
        name = twitchUser.display_name;
        englishName = twitchUser.display_name;
        photo = twitchUser.profile_image_url;

        // Check if already registered under numeric ID
        const existing = await VTuber.findOne({ platform: 'twitch', platformChannelId });
        if (existing) {
          return c.json({ error: 'VTuber already registered', vtuber: existing }, 409);
        }
      } catch (err) {
        return c.json({ error: 'Twitch API lookup failed', detail: String(err) }, 502);
      }
    }

    // Save VTuber
    const vtuber = await VTuber.create({
      name,
      englishName,
      photo,
      platform,
      source,
      platformChannelId,
      org,
      suborg,
      isTracked: true,
      lastSyncedAt: null,
      lastLiveSyncedAt: null,
      lastStatsSyncedAt: null,
    });

    // Fire-and-forget: registration only resolves identity (name/photo/
    // platformChannelId) above, it never touches Stream/Clip — without this,
    // a freshly-registered VTuber sits with empty streams/clips until
    // someone calls POST /api/sync/* by hand. Not awaited so a slow or
    // failing sync doesn't fail registration itself.
    const initialSync = source === 'holodex' ? syncFromHolodex : source === 'youtube_api' ? syncFromYoutube : syncFromTwitch;
    initialSync(vtuber._id.toString(), true).catch((err) =>
      console.error(`Initial sync failed for ${vtuber.englishName}:`, err)
    );

    if (platform === 'twitch') {
      // Fire-and-forget: a subscribe hiccup shouldn't fail registration,
      // and the next EventSub reconnect's reconcile will catch it anyway.
      subscribeToLive(platformChannelId).catch((err) =>
        console.error(`Failed to subscribe to EventSub for ${platformChannelId}:`, err)
      );
    }

    return c.json({ message: 'VTuber registered successfully', vtuber }, 201);
  } catch (error) {
    return c.json({ error: 'Failed to create VTuber', detail: String(error) }, 500);
  }
});

/**
 * GET /api/vtubers
 * List registered VTubers with search/filter
 */
vtubersRoute.get('/api/vtubers', async (c) => {
  try {
    const platform = c.req.query('platform');
    const org = c.req.query('org');
    const isTracked = c.req.query('isTracked');
    const name = c.req.query('name');

    const filter: any = {};
    if (platform) filter.platform = platform;
    if (org) filter.org = org;
    if (name) {
      const pattern = { $regex: escapeRegex(name), $options: 'i' };
      filter.$or = [{ name: pattern }, { englishName: pattern }];
    }
    if (isTracked !== undefined) filter.isTracked = isTracked === 'true';

    const vtubers = await VTuber.find(filter).sort({ name: 1 });
    return c.json(vtubers);
  } catch (error) {
    return c.json({ error: 'Failed to list VTubers', detail: String(error) }, 500);
  }
});

/**
 * GET /api/vtubers/live
 * List VTubers who currently have a live stream. Registered before
 * GET /api/vtubers/:id so "live" isn't swallowed as an :id param.
 */
vtubersRoute.get('/api/vtubers/live', async (c) => {
  try {
    const liveStreams = await Stream.find({ status: 'live' }).sort({ startTime: -1 });

    const vtuberIds = [...new Set(liveStreams.map((s) => s.vtuberId.toString()))];
    const vtubers = await VTuber.find({ _id: { $in: vtuberIds } });
    const vtubersById = new Map(vtubers.map((v) => [v._id.toString(), v]));

    const results = liveStreams
      .map((s) => {
        const vtuber = vtubersById.get(s.vtuberId.toString());
        // Stream outlived its VTuber (deletion doesn't retroactively end streams
        // subscribed before it, though delete does cascade-remove them now).
        if (!vtuber) return null;
        return {
          vtuber,
          stream: {
            // externalId is the only stable per-stream identity here: `url` is
            // constant across a Twitch streamer's streams, and startTime is
            // refreshed on every markLive upsert. Clients diffing live state
            // (`oshihub watch`) need a key that can't drift.
            externalId: s.externalId,
            title: s.title,
            url: s.url,
            thumbnailUrl: s.thumbnailUrl,
            startTime: s.startTime,
          },
        };
      })
      .filter((r) => r !== null);

    return c.json(results);
  } catch (error) {
    return c.json({ error: 'Failed to list live VTubers', detail: String(error) }, 500);
  }
});

/**
 * GET /api/vtubers/:id
 * Retrieve single VTuber detail with associated records
 */
vtubersRoute.get('/api/vtubers/:id', async (c) => {
  try {
    const id = c.req.param('id');
    const vtuber = await VTuber.findById(id);

    if (!vtuber) {
      return c.json({ error: 'VTuber not found' }, 404);
    }

    // Fetch related records
    const streams = await Stream.find({ vtuberId: id }).sort({ startTime: -1 }).limit(10);
    const clips = await Clip.find({ vtuberId: id }).sort({ createdAt: -1 }).limit(10);
    const snapshots = await StatSnapshot.find({ vtuberId: id }).sort({ capturedAt: -1 }).limit(10);

    return c.json({
      vtuber,
      streams,
      clips,
      snapshots,
    });
  } catch (error) {
    return c.json({ error: 'Failed to retrieve VTuber details', detail: String(error) }, 500);
  }
});

/**
 * GET /api/vtubers/:id/profile-url
 * Resolve a browsable channel URL for this VTuber. YouTube's platformChannelId
 * is already the canonical channel ID; Twitch's is a numeric ID that only
 * Helix can translate back to the login name Twitch URLs require.
 */
vtubersRoute.get('/api/vtubers/:id/profile-url', async (c) => {
  try {
    const id = c.req.param('id');
    const vtuber = await VTuber.findById(id);

    if (!vtuber) {
      return c.json({ error: 'VTuber not found' }, 404);
    }

    if (vtuber.platform === 'youtube') {
      return c.json({ url: `https://youtube.com/channel/${vtuber.platformChannelId}` });
    }

    const twitchUser = await fetchTwitchUserById(vtuber.platformChannelId);
    if (!twitchUser) {
      return c.json({ error: 'Could not resolve current Twitch login for this channel' }, 502);
    }

    return c.json({ url: `https://twitch.tv/${twitchUser.login}` });
  } catch (error) {
    return c.json({ error: 'Failed to resolve profile URL', detail: String(error) }, 500);
  }
});

/**
 * The shared bucket-window arithmetic for the /stats/* endpoints: parses
 * unit/buckets query params and computes the window start, aligned to the
 * start of the current UTC bucket (Monday-based for weeks, matching
 * $dateTrunc's startOfWeek). UTC has no DST, so stepping backwards by a
 * fixed ms-per-bucket is safe.
 */
function bucketWindow(c: { req: { query: (k: string) => string | undefined } }) {
  const unit = c.req.query('unit') === 'day' ? ('day' as const) : ('week' as const);
  const bucketsRaw = parseInt(c.req.query('buckets') ?? '', 10);
  const buckets = Number.isFinite(bucketsRaw) ? Math.min(Math.max(bucketsRaw, 1), 366) : 52;

  const stepMs = (unit === 'week' ? 7 : 1) * 24 * 60 * 60 * 1000;
  const currentStart = new Date();
  currentStart.setUTCHours(0, 0, 0, 0);
  if (unit === 'week') {
    currentStart.setUTCDate(currentStart.getUTCDate() - ((currentStart.getUTCDay() + 6) % 7));
  }
  const from = new Date(currentStart.getTime() - (buckets - 1) * stepMs);

  // Dense bucket-start dates ("YYYY-MM-DD", UTC), parallel to whatever
  // per-bucket arrays the endpoint returns — clients label bars with these
  // rather than doing their own calendar math (the Rust CLI deliberately
  // carries no date crate).
  const starts = Array.from({ length: buckets }, (_, i) =>
    new Date(from.getTime() + i * stepMs).toISOString().slice(0, 10)
  );

  return { unit, buckets, stepMs, from, starts };
}

/**
 * GET /api/vtubers/:id/stats/stream-frequency
 * Bucketed stream counts for the dashboard's frequency chart.
 * Query: unit=week|day (default week), buckets=<n> (default 52, max 366).
 * Returns a DENSE zero-filled array, oldest → newest, sized for direct use
 * as sparkline data — the newest bucket is the current (partial) week/day.
 */
vtubersRoute.get('/api/vtubers/:id/stats/stream-frequency', async (c) => {
  try {
    const id = c.req.param('id');
    const vtuber = await VTuber.findById(id);
    if (!vtuber) {
      return c.json({ error: 'VTuber not found' }, 404);
    }

    const { unit, buckets, stepMs, from, starts } = bucketWindow(c);

    // Count only streams that actually happened: 'upcoming' would put phantom
    // counts in future buckets (HoloDex returns scheduled streams), and
    // 'unknown' is unvetted. The { vtuberId, status } index covers this match.
    const grouped = await Stream.aggregate([
      {
        $match: {
          vtuberId: vtuber._id,
          status: { $in: ['live', 'ended'] },
          startTime: { $gte: from },
        },
      },
      {
        $group: {
          _id: { $dateTrunc: { date: '$startTime', unit, startOfWeek: 'monday' } },
          count: { $sum: 1 },
        },
      },
    ]);

    // Oldest stream on record. Buckets that predate it aren't "zero streams",
    // they're "not yet tracking" — returned as null (below) so clients can
    // render absent differently from a genuinely quiet week, without having
    // to do their own date math. No streams at all means every bucket is null.
    const firstStream = await Stream.findOne({
      vtuberId: vtuber._id,
      status: { $in: ['live', 'ended'] },
    }).sort({ startTime: 1 });
    const firstStreamMs = firstStream ? firstStream.startTime.getTime() : Infinity;

    // $group only emits buckets that have data; densify so a quiet week shows
    // as an explicit 0 instead of silently missing from the series.
    const byTime = new Map<number, number>(
      grouped.map((g) => [new Date(g._id).getTime(), g.count])
    );
    const counts = Array.from({ length: buckets }, (_, i): number | null => {
      const bucketStart = from.getTime() + i * stepMs;
      // A bucket that ends on/before the first recorded stream contains no
      // tracked time at all; the bucket *containing* it stays numeric.
      if (bucketStart + stepMs <= firstStreamMs) return null;
      return byTime.get(bucketStart) ?? 0;
    });

    return c.json({
      unit,
      from: from.toISOString(),
      firstStreamAt: firstStream ? firstStream.startTime.toISOString() : null,
      counts,
      starts,
    });
  } catch (error) {
    return c.json({ error: 'Failed to compute stream frequency', detail: String(error) }, 500);
  }
});

/**
 * GET /api/vtubers/:id/stats/subscriber-trend
 * Daily subscriber/follower series for the dashboard's trend chart.
 * Query: days=<n> (default 90, max 365).
 *
 * Unlike stream-frequency's counts, points here are SPARSE — a day with no
 * snapshot is simply missing (it means "didn't sync", never "zero
 * subscribers"), and the line connects across the gap. Each point carries an
 * integer `day` offset from `from` so clients keep gaps proportional on the
 * x-axis without doing their own date math, plus its date for labelling.
 */
vtubersRoute.get('/api/vtubers/:id/stats/subscriber-trend', async (c) => {
  try {
    const id = c.req.param('id');
    const vtuber = await VTuber.findById(id);
    if (!vtuber) {
      return c.json({ error: 'VTuber not found' }, 404);
    }

    const daysRaw = parseInt(c.req.query('days') ?? '', 10);
    const days = Number.isFinite(daysRaw) ? Math.min(Math.max(daysRaw, 1), 365) : 90;

    const dayMs = 24 * 60 * 60 * 1000;
    const currentDayStart = new Date();
    currentDayStart.setUTCHours(0, 0, 0, 0);
    const from = new Date(currentDayStart.getTime() - (days - 1) * dayMs);

    // One point per UTC day, the day's LAST snapshot winning — forced syncs
    // (stream.online, manual s) can capture several per day and the newest
    // is simply the freshest truth. $last is only meaningful because of the
    // $sort feeding the $group. The { vtuberId, capturedAt } index covers
    // the match.
    const grouped = await StatSnapshot.aggregate([
      { $match: { vtuberId: vtuber._id, capturedAt: { $gte: from } } },
      { $sort: { capturedAt: 1 } },
      {
        $group: {
          _id: { $dateTrunc: { date: '$capturedAt', unit: 'day' } },
          subscribers: { $last: '$subscriberCount' },
        },
      },
      { $sort: { _id: 1 } },
    ]);

    const points = grouped.map((g) => {
      const dayStart = new Date(g._id);
      return {
        day: Math.round((dayStart.getTime() - from.getTime()) / dayMs),
        date: dayStart.toISOString().slice(0, 10),
        subscribers: g.subscribers,
      };
    });

    // Deltas are "current vs where they were at least N days ago" — the
    // newest snapshot that's ≥N days old, not an interpolation. null when
    // history is too short to answer honestly (a young record shouldn't
    // report a fake +0).
    const current = points.length > 0 ? points[points.length - 1].subscribers : null;
    const deltaVs = async (msAgo: number) => {
      if (current === null) return null;
      const baseline = await StatSnapshot.findOne({
        vtuberId: vtuber._id,
        capturedAt: { $lte: new Date(Date.now() - msAgo) },
      }).sort({ capturedAt: -1 });
      return baseline ? current - baseline.subscriberCount : null;
    };

    return c.json({
      from: from.toISOString().slice(0, 10),
      days,
      points,
      current,
      delta7d: await deltaVs(7 * dayMs),
      delta30d: await deltaVs(30 * dayMs),
    });
  } catch (error) {
    return c.json({ error: 'Failed to compute subscriber trend', detail: String(error) }, 500);
  }
});

// Below this, a "stream" doesn't count for duration stats. YouTube-sourced
// Stream docs include Shorts and ordinary uploads (observed: 14–46 second
// "streams" on real channels), which would poison any average; no genuine
// live stream is shorter than this.
const MIN_STREAM_DURATION_SECS = 600;

// Plain JS median rather than Mongo's $median accumulator — that needs
// MongoDB 7.0+, and coupling the endpoint to the Atlas version isn't worth
// it for a ten-line function over a handful of values per bucket.
function median(values: number[]): number | null {
  if (values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[mid] : Math.round((sorted[mid - 1] + sorted[mid]) / 2);
}

/**
 * GET /api/vtubers/:id/stats/duration-trend
 * Median stream duration (seconds) per bucket, for the dashboard.
 * Query: unit=week|day (default week), buckets=<n> (default 52, max 366).
 *
 * MEDIAN, not mean — buckets hold 3–8 streams, where one 12h subathon
 * would double a mean, and it pairs with the MIN_STREAM_DURATION_SECS
 * floor to survive the Shorts contamination described above. Buckets with
 * no qualifying stream are null (an "average duration" of nothing isn't
 * 0). Dense arrays parallel to `starts`, same window arithmetic as
 * stream-frequency so the two charts' buckets line up column-for-column.
 */
vtubersRoute.get('/api/vtubers/:id/stats/duration-trend', async (c) => {
  try {
    const id = c.req.param('id');
    const vtuber = await VTuber.findById(id);
    if (!vtuber) {
      return c.json({ error: 'VTuber not found' }, 404);
    }

    const { unit, buckets, stepMs, from, starts } = bucketWindow(c);

    // status 'ended' only: live streams have null duration anyway, and a
    // still-running stream has no final duration to count. $gte also
    // excludes null durations (EventSub-ended streams no VOD sync has
    // backfilled yet).
    const grouped = await Stream.aggregate([
      {
        $match: {
          vtuberId: vtuber._id,
          status: 'ended',
          duration: { $gte: MIN_STREAM_DURATION_SECS },
          startTime: { $gte: from },
        },
      },
      {
        $group: {
          _id: { $dateTrunc: { date: '$startTime', unit, startOfWeek: 'monday' } },
          durations: { $push: '$duration' },
        },
      },
    ]);

    const byTime = new Map<number, number[]>(
      grouped.map((g) => [new Date(g._id).getTime(), g.durations])
    );
    const medians = Array.from({ length: buckets }, (_, i): number | null =>
      median(byTime.get(from.getTime() + i * stepMs) ?? [])
    );
    const counts = Array.from(
      { length: buckets },
      (_, i) => (byTime.get(from.getTime() + i * stepMs) ?? []).length
    );

    const all = grouped.flatMap((g) => g.durations as number[]);

    return c.json({
      unit,
      from: from.toISOString(),
      starts,
      medians,
      counts,
      overallMedian: median(all),
      longest: all.length > 0 ? Math.max(...all) : null,
    });
  } catch (error) {
    return c.json({ error: 'Failed to compute duration trend', detail: String(error) }, 500);
  }
});

/**
 * PUT /api/vtubers/:id
 * Update VTuber details (e.g. name, isTracked status)
 */
vtubersRoute.put('/api/vtubers/:id', async (c) => {
  try {
    const id = c.req.param('id');
    const body = await c.req.json();

    const UpdateSchema = z.object({
      name: z.string().min(1).optional(),
      englishName: z.string().min(1).optional(),
      photo: z.string().url().optional(),
      isTracked: z.boolean().optional(),
      org: z.string().optional(),
      suborg: z.string().optional(),
    });

    const parsed = UpdateSchema.safeParse(body);
    if (!parsed.success) {
      return c.json({ error: 'Invalid request body', details: parsed.error.format() }, 400);
    }

    const updated = await VTuber.findByIdAndUpdate(id, parsed.data, { new: true });

    if (!updated) {
      return c.json({ error: 'VTuber not found' }, 404);
    }

    return c.json({ message: 'VTuber updated successfully', vtuber: updated });
  } catch (error) {
    return c.json({ error: 'Failed to update VTuber', detail: String(error) }, 500);
  }
});

/**
 * DELETE /api/vtubers/:id
 * Delete a VTuber and their associated streams, clips, snapshots
 */
vtubersRoute.delete('/api/vtubers/:id', async (c) => {
  try {
    const id = c.req.param('id');
    const deleted = await VTuber.findByIdAndDelete(id);

    if (!deleted) {
      return c.json({ error: 'VTuber not found' }, 404);
    }

    // Cascade deletion of dependent records
    await Stream.deleteMany({ vtuberId: id });
    await Clip.deleteMany({ vtuberId: id });
    await StatSnapshot.deleteMany({ vtuberId: id });

    if (deleted.platform === 'twitch') {
      await unsubscribeFromLive(deleted.platformChannelId).catch((err) =>
        console.error(`Failed to remove EventSub subscription for ${deleted.platformChannelId}:`, err)
      );
    }

    return c.json({ message: 'VTuber and all associated data deleted successfully', vtuber: deleted });
  } catch (error) {
    return c.json({ error: 'Failed to delete VTuber', detail: String(error) }, 500);
  }
});
