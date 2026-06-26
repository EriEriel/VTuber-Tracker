// Minimal in-memory token cache for MVP/testing purposes.
// This is intentionally module-level state, not stored in MongoDB —
// the app access token is shared across ALL Twitch-sourced VTubers,
// it is not per-channel, so it doesn't belong on any one document.

let cachedToken: string | null = null;
let tokenExpiresAt: number = 0; // epoch ms

export async function getValidTwitchToken(): Promise<string> {
  const now = Date.now();

  // 60s buffer so we never hand out a token that expires mid-request
  if (cachedToken && now < tokenExpiresAt - 60_000) {
    return cachedToken;
  }

  const clientId = process.env.TWITCH_CLIENT_ID;
  const clientSecret = process.env.TWITCH_CLIENT_SECRET;

  if (!clientId || !clientSecret) {
    throw new Error('TWITCH_CLIENT_ID or TWITCH_CLIENT_SECRET is not set in .env');
  }

  const res = await fetch('https://id.twitch.tv/oauth2/token', {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({
      client_id: clientId,
      client_secret: clientSecret,
      grant_type: 'client_credentials',
    }),
  });

  if (!res.ok) {
    const body = await res.text();
    throw new Error(`Twitch token request failed: ${res.status} ${body}`);
  }

  const data = await res.json() as { access_token: string; expires_in: number };

  cachedToken = data.access_token;
  tokenExpiresAt = now + data.expires_in * 1000;

  return cachedToken;
}
