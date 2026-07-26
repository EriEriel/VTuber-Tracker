import { Hono } from 'hono';
import { createHash, createHmac, timingSafeEqual } from 'node:crypto';
import { handleNotification, requireEventSubSecret } from '../lib/twitch-eventsub';

// Twitch's EventSub webhook callback. Deliberately NOT behind requireApiToken
// (mounted outside /api and /sync in index.ts) — Twitch calls this directly
// and cannot carry our bearer token. Its safety instead rests entirely on
// the HMAC signature check below. See LIVE_DETECTION.md (Phase 1) for the
// full design, especially the signature-verification traps.
export const eventsubRoute = new Hono();

const MAX_MESSAGE_AGE_MS = 10 * 60 * 1000;

// Small in-memory dedupe window: Twitch retries on any non-2xx response and
// will legitimately resend the same message-id. Not persisted -- losing this
// on a restart is fine, since a resend after a restart is still just a resend.
const seenMessageIds = new Map<string, number>();

function pruneSeenMessageIds(now: number): void {
  for (const [id, seenAt] of seenMessageIds) {
    if (now - seenAt > MAX_MESSAGE_AGE_MS) seenMessageIds.delete(id);
  }
}

// Hash both sides to equal length before comparing -- same reasoning as
// require-api-token.ts: timingSafeEqual throws outright on a length
// mismatch, and a forged signature header of the wrong length would
// otherwise never even reach the timing-safe comparison.
function signaturesMatch(expected: string, provided: string): boolean {
  const a = createHash('sha256').update(expected).digest();
  const b = createHash('sha256').update(provided).digest();
  return timingSafeEqual(a, b);
}

eventsubRoute.post('/eventsub/callback', async (c) => {
  const messageId = c.req.header('Twitch-Eventsub-Message-Id');
  const timestamp = c.req.header('Twitch-Eventsub-Message-Timestamp');
  const signature = c.req.header('Twitch-Eventsub-Message-Signature');
  const messageType = c.req.header('Twitch-Eventsub-Message-Type');

  if (!messageId || !timestamp || !signature || !messageType) {
    return c.json({ error: 'Missing required EventSub headers' }, 400);
  }

  const messageAge = Date.now() - new Date(timestamp).getTime();
  if (Number.isNaN(messageAge) || messageAge > MAX_MESSAGE_AGE_MS) {
    return c.json({ error: 'Stale or invalid timestamp' }, 403);
  }

  // The raw body, byte for byte -- JSON.parse then JSON.stringify would not
  // reproduce it and the HMAC would never match. Read as text and verify
  // before ever parsing it as JSON.
  const rawBody = await c.req.text();

  const secret = requireEventSubSecret();
  const expectedSignature =
    'sha256=' + createHmac('sha256', secret).update(messageId + timestamp + rawBody).digest('hex');

  if (!signaturesMatch(expectedSignature, signature)) {
    return c.json({ error: 'Invalid signature' }, 403);
  }

  const now = Date.now();
  pruneSeenMessageIds(now);
  if (seenMessageIds.has(messageId)) {
    return c.body(null, 204);
  }
  seenMessageIds.set(messageId, now);

  if (messageType === 'webhook_callback_verification') {
    const body = JSON.parse(rawBody) as { challenge: string };
    return c.text(body.challenge, 200);
  }

  if (messageType === 'revocation') {
    console.warn('EventSub subscription revoked:', JSON.parse(rawBody).subscription);
    return c.body(null, 204);
  }

  if (messageType === 'notification') {
    // Twitch expects a 2xx within 10 seconds and retries otherwise. The
    // stream.online handler calls syncFromTwitch(), a multi-second fan-out
    // (bug-report.md #10) -- so respond first, then handle fire-and-forget.
    handleNotification(JSON.parse(rawBody)).catch((err) =>
      console.error('Failed to handle EventSub notification:', err)
    );
    return c.body(null, 204);
  }

  return c.body(null, 204);
});
