// Real-time "went live"/"went offline" notifications for Twitch-sourced
// VTubers via EventSub over webhook transport. See LIVE_DETECTION.md (Phase 1)
// at the repo root for the full design rationale, including why this
// replaced websocket transport: websocket's max_total_cost of 10 (2 per
// VTuber for stream.online + stream.offline) hard-capped tracking at 5
// Twitch VTubers, silently dropping the 6th. Webhook's max_total_cost is
// 10,000. TWITCH_EVENTSUB.md is superseded -- its protocol detail is still
// accurate, its transport-choice argument is not.
//
// Subscription management (create/list/delete) now authenticates with the
// **app** access token (getValidTwitchToken, shared with the rest of
// sync.ts) rather than a user token -- webhook transport doesn't need one.
// This is what let twitch-user-token.ts and routes/auth.ts be deleted.
//
// EventSub is only the trigger: on `stream.online` we don't try to build a
// Stream document from the event payload (it's intentionally minimal — no
// title, no thumbnail) — we call the existing syncFromTwitch() so the
// already-tested Helix -> Mongo pipeline does the real work. `stream.offline`
// carries even less (no timestamp at all), so it goes through markEnded()
// directly rather than syncFromTwitch(), stamping endTime on arrival.
//
// The actual HTTP callback lives in routes/eventsub.ts (signature
// verification, fast-204-then-fire-and-forget) and calls handleNotification()
// here after verifying the request is genuinely from Twitch.

import { VTuber } from '../models';
import { getValidTwitchToken } from './twitch-token';
import { syncFromTwitch } from './sync';
import { markEnded } from './live-state';

const EVENT_TYPES = ['stream.online', 'stream.offline'] as const;
type EventType = (typeof EVENT_TYPES)[number];

function requireClientId(): string {
  const id = process.env.TWITCH_CLIENT_ID;
  if (!id) throw new Error('TWITCH_CLIENT_ID is not set in .env');
  return id;
}

export function requireEventSubSecret(): string {
  const secret = process.env.EVENTSUB_SECRET;
  if (!secret) throw new Error('EVENTSUB_SECRET is not set in .env — required for EventSub webhook subscriptions');
  return secret;
}

// Twitch requires HTTPS on port 443 for webhook callbacks -- anything else
// is rejected at subscription-creation time, so fail fast here instead.
function requirePublicUrl(): string {
  const raw = process.env.PUBLIC_URL;
  if (!raw) throw new Error('PUBLIC_URL is not set in .env');

  const parsed = new URL(raw);
  if (parsed.protocol !== 'https:' || (parsed.port && parsed.port !== '443')) {
    throw new Error(`PUBLIC_URL must be HTTPS on port 443 (Twitch rejects anything else): ${raw}`);
  }
  return raw.replace(/\/+$/, '');
}

async function helixFetch(path: string, init: RequestInit = {}): Promise<Response> {
  const token = await getValidTwitchToken();
  return fetch(`https://api.twitch.tv/helix${path}`, {
    ...init,
    headers: {
      ...init.headers,
      Authorization: `Bearer ${token}`,
      'Client-Id': requireClientId(),
    },
  });
}

async function createSubscription(broadcasterId: string, type: EventType): Promise<void> {
  const secret = requireEventSubSecret();
  const callback = `${requirePublicUrl()}/eventsub/callback`;

  const res = await helixFetch('/eventsub/subscriptions', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      type,
      version: '1',
      condition: { broadcaster_user_id: broadcasterId },
      transport: { method: 'webhook', callback, secret },
    }),
  });

  // 409 = a subscription for this broadcaster/type/transport already
  // exists (e.g. reconcile ran twice) — not an error, just a no-op.
  if (!res.ok && res.status !== 409) {
    console.error(`Failed to subscribe to ${type} for ${broadcasterId}: ${res.status} ${await res.text()}`);
  }
}

interface HelixSubscription {
  id: string;
  status: string;
  type: string;
  condition: { broadcaster_user_id?: string };
}

// No `type` filter: this app only ever creates stream.online/stream.offline
// subscriptions, and the list endpoint is already scoped to this app's
// Client-Id, so one call covers both.
async function listSubscriptions(): Promise<HelixSubscription[]> {
  const res = await helixFetch('/eventsub/subscriptions');
  if (!res.ok) {
    console.error(`Failed to list EventSub subscriptions: ${res.status} ${await res.text()}`);
    return [];
  }
  const data = (await res.json()) as { data: HelixSubscription[] };
  return data.data ?? [];
}

async function deleteSubscription(subscriptionId: string): Promise<void> {
  await helixFetch(`/eventsub/subscriptions?id=${subscriptionId}`, { method: 'DELETE' });
}

// Terminal: Twitch will never deliver events for these again, so they must
// be deleted and recreated. `websocket_disconnected` is included too: this
// codebase no longer creates websocket-transport subscriptions at all, so
// that status can now only mean a leftover from before this migration --
// dead weight to sweep on the first reconcile after deploy.
// `webhook_callback_verification_pending` is deliberately NOT here -- it's
// mid-handshake (Twitch hasn't yet GET'd the challenge), and deleting it
// would race a subscription that's about to become `enabled` on its own.
const TERMINAL_SUBSCRIPTION_STATUSES = new Set([
  'webhook_callback_verification_failed',
  'notification_failures_exceeded',
  'websocket_disconnected',
]);

/**
 * Diff Twitch's actual subscription list against currently-tracked Twitch
 * VTubers: drop subscriptions for anyone no longer tracked (or terminally
 * failed), add subscriptions for anyone tracked but missing one. Subscriptions
 * are bound to the callback URL rather than a session, so — unlike websocket
 * transport — they survive backend restarts; this still runs on boot to
 * self-heal drift, but it's no longer load-bearing.
 */
async function reconcileSubscriptions(): Promise<void> {
  const [trackedVtubers, existingSubs] = await Promise.all([
    VTuber.find({ platform: 'twitch', isTracked: true }),
    listSubscriptions(),
  ]);

  const trackedIds = new Set(trackedVtubers.map((v) => v.platformChannelId));

  await Promise.all(
    existingSubs
      .filter(
        (sub) =>
          !sub.condition.broadcaster_user_id ||
          !trackedIds.has(sub.condition.broadcaster_user_id) ||
          TERMINAL_SUBSCRIPTION_STATUSES.has(sub.status)
      )
      .map((sub) => deleteSubscription(sub.id))
  );

  // Coverage is per (broadcaster, event type) — a VTuber can have
  // stream.online covered but not stream.offline (or vice versa). A pending
  // verification counts as coverage too, so it isn't raced by a duplicate
  // create while Twitch is still completing the handshake.
  const covered = new Set(
    existingSubs
      .filter((sub) => sub.status === 'enabled' || sub.status === 'webhook_callback_verification_pending')
      .map((sub) => `${sub.condition.broadcaster_user_id}:${sub.type}`)
  );

  const toCreate: Promise<void>[] = [];
  for (const v of trackedVtubers) {
    for (const type of EVENT_TYPES) {
      if (!covered.has(`${v.platformChannelId}:${type}`)) {
        toCreate.push(createSubscription(v.platformChannelId, type));
      }
    }
  }
  await Promise.all(toCreate);
}

export async function handleNotification(payload: any): Promise<void> {
  const type = payload.subscription?.type;
  const broadcasterId = payload.event?.broadcaster_user_id;
  if (!broadcasterId) return;

  const vtuber = await VTuber.findOne({ platform: 'twitch', platformChannelId: broadcasterId });
  if (!vtuber) return;

  if (type === 'stream.online') {
    console.log(`${vtuber.name} just went live on Twitch — syncing`);
    await syncFromTwitch(vtuber._id.toString(), true);
  } else if (type === 'stream.offline') {
    const ended = await markEnded(vtuber._id);
    if (ended) {
      console.log(`${vtuber.name} went offline on Twitch — marked stream ended`);
    }
  }
}

/**
 * Run once at boot: reconciles EventSub subscription coverage against
 * tracked Twitch VTubers. Unlike the old websocket listener, this doesn't
 * hold any connection open -- Twitch delivers events by calling
 * routes/eventsub.ts directly, whenever it wants, for as long as the
 * subscription exists.
 */
export function initTwitchEventSub(): void {
  if (!process.env.TWITCH_CLIENT_ID) {
    console.warn('TWITCH_CLIENT_ID not set — skipping Twitch EventSub reconciliation');
    return;
  }
  if (!process.env.EVENTSUB_SECRET || !process.env.PUBLIC_URL) {
    console.warn('EVENTSUB_SECRET or PUBLIC_URL not set — skipping Twitch EventSub reconciliation');
    return;
  }

  reconcileSubscriptions().catch((err) => console.error('EventSub subscription reconciliation failed:', err));
}

export async function subscribeToLive(broadcasterId: string): Promise<void> {
  await Promise.all(EVENT_TYPES.map((type) => createSubscription(broadcasterId, type)));
}

export async function unsubscribeFromLive(broadcasterId: string): Promise<void> {
  const existing = await listSubscriptions();
  const matches = existing.filter((sub) => sub.condition.broadcaster_user_id === broadcasterId);
  await Promise.all(matches.map((sub) => deleteSubscription(sub.id)));
}
