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
  try {
    const force = c.req.query('force') === 'true';

    console.log(`Starting global sync (force=${force})...`);
    const holodexResults = await syncFromHolodex(undefined, force);
    const youtubeResults = await syncFromYoutube(undefined, force);
    const twitchResults = await syncFromTwitch(undefined, force);

    return c.json({
      success: true,
      summary: {
        holodex: { count: holodexResults.length, details: holodexResults },
        youtube: { count: youtubeResults.length, details: youtubeResults },
        twitch: { count: twitchResults.length, details: twitchResults },
      }
    });
  } catch (error) {
    return c.json({ error: 'Global sync failed', detail: String(error) }, 500);
  }
});
