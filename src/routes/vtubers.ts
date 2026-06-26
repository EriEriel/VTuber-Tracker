import { Hono } from 'hono';
import { z } from 'zod';
import { VTuber, Stream, Clip, StatSnapshot } from '../models';
import { resolveTwitchUser } from '../lib/sync';

export const vtubersRoute = new Hono();

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
    let source: 'holodex' | 'youtube_api' | 'twitch_api';
    let platformChannelId = channelId;
    let org = undefined;
    let suborg = undefined;

    if (platform === 'youtube') {
      // 1. Try HoloDex first
      const holodexKey = process.env.HOLODEX_API_KEY;
      let holodexSuccess = false;

      if (holodexKey) {
        try {
          const holodexUrl = `https://holodex.net/api/v2/channels/${channelId}`;
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

    const filter: any = {};
    if (platform) filter.platform = platform;
    if (org) filter.org = org;
    if (isTracked !== undefined) filter.isTracked = isTracked === 'true';

    const vtubers = await VTuber.find(filter).sort({ name: 1 });
    return c.json(vtubers);
  } catch (error) {
    return c.json({ error: 'Failed to list VTubers', detail: String(error) }, 500);
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

    return c.json({ message: 'VTuber and all associated data deleted successfully', vtuber: deleted });
  } catch (error) {
    return c.json({ error: 'Failed to delete VTuber', detail: String(error) }, 500);
  }
});
