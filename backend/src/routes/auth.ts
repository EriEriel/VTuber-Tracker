import { Hono } from 'hono';
import { TwitchAuth } from '../models';
import { TWITCH_OAUTH_REDIRECT_URI } from '../lib/twitch-user-token';

export const authRoute = new Hono();

/**
 * GET /auth/twitch/login
 * One-time interactive step: redirects to Twitch's authorize page so a
 * real user can grant a user access token (required for EventSub's
 * websocket transport — see TWITCH_EVENTSUB.md). No scopes requested,
 * since stream.online is a public topic.
 */
authRoute.get('/auth/twitch/login', (c) => {
  const clientId = process.env.TWITCH_CLIENT_ID;
  if (!clientId) {
    return c.json({ error: 'TWITCH_CLIENT_ID is not set in .env' }, 500);
  }

  const params = new URLSearchParams({
    client_id: clientId,
    redirect_uri: TWITCH_OAUTH_REDIRECT_URI,
    response_type: 'code',
    scope: '',
  });

  return c.redirect(`https://id.twitch.tv/oauth2/authorize?${params}`);
});

/**
 * GET /auth/twitch/callback
 * Exchanges the authorization code for an access + refresh token pair and
 * persists them (TwitchAuth, a singleton document) so getValidUserToken()
 * can use and refresh them without repeating this flow.
 */
authRoute.get('/auth/twitch/callback', async (c) => {
  const code = c.req.query('code');
  const error = c.req.query('error');

  if (error) {
    return c.json({ error: `Twitch authorization failed: ${error}` }, 400);
  }
  if (!code) {
    return c.json({ error: 'Missing code query param' }, 400);
  }

  const clientId = process.env.TWITCH_CLIENT_ID;
  const clientSecret = process.env.TWITCH_CLIENT_SECRET;
  if (!clientId || !clientSecret) {
    return c.json({ error: 'TWITCH_CLIENT_ID or TWITCH_CLIENT_SECRET is not set in .env' }, 500);
  }

  const res = await fetch('https://id.twitch.tv/oauth2/token', {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({
      client_id: clientId,
      client_secret: clientSecret,
      code,
      grant_type: 'authorization_code',
      redirect_uri: TWITCH_OAUTH_REDIRECT_URI,
    }),
  });

  if (!res.ok) {
    const body = await res.text();
    return c.json({ error: 'Failed to exchange code for tokens', detail: body }, 502);
  }

  const data = (await res.json()) as { access_token: string; refresh_token: string; expires_in: number };

  await TwitchAuth.findOneAndUpdate(
    {},
    {
      accessToken: data.access_token,
      refreshToken: data.refresh_token,
      expiresAt: new Date(Date.now() + data.expires_in * 1000),
    },
    { upsert: true }
  );

  return c.json({ message: 'Twitch account authorized. EventSub can now subscribe over websocket.' });
});
