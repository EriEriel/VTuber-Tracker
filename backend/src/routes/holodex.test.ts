import { Hono } from 'hono';

export const holodexTestRoute = new Hono();

// Usada Pekora's channel ID — well-known HoloDex-tracked talent, good default test subject
const TEST_CHANNEL_ID = 'UC1DCedRgGHBdm81E1llLhOQ';

holodexTestRoute.get('/sync/test/holodex', async (c) => {
  const apiKey = process.env.HOLODEX_API_KEY;

  if (!apiKey) {
    return c.json({ error: 'HOLODEX_API_KEY is not set in .env' }, 500);
  }

  const url = `https://holodex.net/api/v2/videos?channel_id=${TEST_CHANNEL_ID}&limit=5`;

  try {
    const res = await fetch(url, {
      headers: {
        'X-APIKEY': apiKey,
      },
    });

    if (!res.ok) {
      // Surface the actual status + body so a bad key or bad query shows up clearly,
      // rather than just "fetch failed"
      const body = await res.text();
      return c.json(
        { error: `HoloDex responded with ${res.status}`, body },
        502,
      );
    }

    const data = await res.json();
    return c.json({ source: 'holodex', count: Array.isArray(data) ? data.length : null, data });
  } catch (err) {
    return c.json({ error: 'Fetch to HoloDex failed', detail: String(err) }, 500);
  }
});
