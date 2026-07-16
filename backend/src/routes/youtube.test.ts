import { Hono } from 'hono';

export const youtubeTestRoute = new Hono();

// Same test channel as HoloDex, so you can compare what each API returns
// for the *same* underlying channel side by side
const TEST_CHANNEL_ID = 'UC1DCedRgGHBdm81E1llLhOQ';

youtubeTestRoute.get('/sync/test/youtube', async (c) => {
  const apiKey = process.env.YOUTUBE_API_KEY;

  if (!apiKey) {
    return c.json({ error: 'YOUTUBE_API_KEY is not set in .env' }, 500);
  }

  // Direct channel lookup by ID — cheap (1 unit), unlike search (100 units).
  // Always prefer this form over /search when you already know the channel ID.
  const url = `https://www.googleapis.com/youtube/v3/channels?part=snippet,statistics&id=${TEST_CHANNEL_ID}&key=${apiKey}`;

  try {
    const res = await fetch(url);

    if (!res.ok) {
      const body = await res.text();
      return c.json(
        { error: `YouTube responded with ${res.status}`, body },
        502,
      );
    }

    const data = await res.json();
    return c.json({ source: 'youtube', data });
  } catch (err) {
    return c.json({ error: 'Fetch to YouTube failed', detail: String(err) }, 500);
  }
});
