// User access token for EventSub's websocket transport. Unlike webhook
// transport (which uses the app access token in twitch-token.ts),
// websocket transport requires a user token even for scope-less topics
// like stream.online — see TWITCH_EVENTSUB.md for why. Obtained once via
// GET /auth/twitch/login (routes/auth.ts), then refreshed automatically
// here using the stored refresh token.

import { TwitchAuth } from '../models';

const REDIRECT_URI = 'http://localhost:3000/auth/twitch/callback';

export async function getValidUserToken(): Promise<string> {
  const clientId = process.env.TWITCH_CLIENT_ID;
  const clientSecret = process.env.TWITCH_CLIENT_SECRET;
  if (!clientId || !clientSecret) {
    throw new Error('TWITCH_CLIENT_ID or TWITCH_CLIENT_SECRET is not set in .env');
  }

  const stored = await TwitchAuth.findOne({});
  if (!stored) {
    throw new Error(`No Twitch user token on file — visit http://localhost:3000/auth/twitch/login to authorize once.`);
  }

  // 60s buffer so we never hand out a token that expires mid-request.
  if (stored.expiresAt.getTime() > Date.now() + 60_000) {
    return stored.accessToken;
  }

  const res = await fetch('https://id.twitch.tv/oauth2/token', {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({
      client_id: clientId,
      client_secret: clientSecret,
      grant_type: 'refresh_token',
      refresh_token: stored.refreshToken,
    }),
  });

  if (!res.ok) {
    const body = await res.text();
    throw new Error(
      `Failed to refresh Twitch user token: ${res.status} ${body}. Re-authorize at http://localhost:3000/auth/twitch/login`
    );
  }

  const data = (await res.json()) as { access_token: string; refresh_token: string; expires_in: number };

  // Twitch rotates the refresh token on every use — the old one stops
  // working, so the new one must be persisted, not just the access token.
  stored.accessToken = data.access_token;
  stored.refreshToken = data.refresh_token;
  stored.expiresAt = new Date(Date.now() + data.expires_in * 1000);
  await stored.save();

  return stored.accessToken;
}

export { REDIRECT_URI as TWITCH_OAUTH_REDIRECT_URI };
