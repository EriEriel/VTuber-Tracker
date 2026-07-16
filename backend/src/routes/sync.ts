import { Hono } from 'hono';
import { syncFromHolodex, syncFromYoutube, syncFromTwitch } from '../lib/sync';

export const syncRoute = new Hono();

/**
 * POST /api/sync/holodex
 * Sync VTubers who are sourced from HoloDex
 */
syncRoute.post('/api/sync/holodex', async (c) => {
  try {
    const id = c.req.query('id'); // optional vtuber ID
    const force = c.req.query('force') === 'true'; // bypass staleness gate

    const results = await syncFromHolodex(id, force);
    return c.json({ source: 'holodex', results });
  } catch (error) {
    return c.json({ error: 'HoloDex sync failed', detail: String(error) }, 500);
  }
});

/**
 * POST /api/sync/youtube
 * Sync VTubers who are sourced from YouTube Data API v3
 */
syncRoute.post('/api/sync/youtube', async (c) => {
  try {
    const id = c.req.query('id');
    const force = c.req.query('force') === 'true';

    const results = await syncFromYoutube(id, force);
    return c.json({ source: 'youtube_api', results });
  } catch (error) {
    return c.json({ error: 'YouTube API sync failed', detail: String(error) }, 500);
  }
});

/**
 * POST /api/sync/twitch
 * Sync VTubers who are sourced from Twitch Helix API
 */
syncRoute.post('/api/sync/twitch', async (c) => {
  try {
    const id = c.req.query('id');
    const force = c.req.query('force') === 'true';

    const results = await syncFromTwitch(id, force);
    return c.json({ source: 'twitch_api', results });
  } catch (error) {
    return c.json({ error: 'Twitch API sync failed', detail: String(error) }, 500);
  }
});

/**
 * POST /api/sync/all
 * Sequentially trigger sync for all sources
 */
syncRoute.post('/api/sync/all', async (c) => {
  const force = c.req.query('force') === 'true';

  const [holodex, youtube, twitch] = await Promise.allSettled([
    syncFromHolodex(undefined, force),
    syncFromYoutube(undefined, force),
    syncFromTwitch(undefined, force),
  ]);

  const summarize = (result: PromiseSettledResult<any>) =>
    result.status === 'fulfilled'
      ? { ok: true, count: result.value.length, details: result.value }
      : { ok: false, error: String(result.reason) };

  return c.json({
    summary: {
      holodex: summarize(holodex),
      youtube: summarize(youtube),
      twitch: summarize(twitch),
    },
  });
});
