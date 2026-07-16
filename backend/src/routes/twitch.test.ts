import { Hono } from 'hono';
import { getValidTwitchToken } from '../lib/twitch-token';

export const twitchTestRoute = new Hono();

// "TwitchDev" — Twitch's own official test/demo account. Always exists,
// always has at least some data, good for a pure connectivity test.
// Swap to a real VTuber's Twitch login once this confirms the pipe works.
const TEST_LOGIN = 'tawffie';

twitchTestRoute.get('/sync/test/twitch', async (c) => {
  const clientId = process.env.TWITCH_CLIENT_ID;

  if (!clientId) {
    return c.json({ error: 'TWITCH_CLIENT_ID is not set in .env' }, 500);
  }

  try {
    const token = await getValidTwitchToken();

    // Step 1: resolve login name -> numeric user_id.
    // Twitch's video/clip endpoints need the ID, not the login string.
    const userRes = await fetch(
      `https://api.twitch.tv/helix/users?login=${TEST_LOGIN}`,
      {
        headers: {
          Authorization: `Bearer ${token}`,
          'Client-Id': clientId,
        },
      },
    );

    if (!userRes.ok) {
      const body = await userRes.text();
      return c.json(
        { error: `Twitch /users responded with ${userRes.status}`, body },
        502,
      );
    }

    const userData = await userRes.json() as { data: Array<{ id: string }> };
    const userId = userData.data[0]?.id;

    if (!userId) {
      return c.json({ error: `No Twitch user found for login "${TEST_LOGIN}"` }, 404);
    }

    // Step 2: fetch videos (VODs) for that user_id
    const videosRes = await fetch(
      `https://api.twitch.tv/helix/videos?user_id=${userId}&first=5`,
      {
        headers: {
          Authorization: `Bearer ${token}`,
          'Client-Id': clientId,
        },
      },
    );

    if (!videosRes.ok) {
      const body = await videosRes.text();
      return c.json(
        { error: `Twitch /videos responded with ${videosRes.status}`, body },
        502,
      );
    }

    const videosData = await videosRes.json();

    return c.json({ source: 'twitch', userId, videos: videosData });
  } catch (err) {
    return c.json({ error: 'Twitch sync test failed', detail: String(err) }, 500);
  }
});
