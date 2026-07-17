// Real-time "went live" notifications for Twitch-sourced VTubers via
// EventSub over WebSocket. See TWITCH_EVENTSUB.md at the repo root for the
// full design rationale (why websocket over webhook, limits, etc).
//
// EventSub is only the trigger: on `stream.online` we don't try to build a
// Stream document from the event payload (it's intentionally minimal — no
// title, no thumbnail) — we call the existing syncFromTwitch() so the
// already-tested Helix -> Mongo pipeline does the real work.

import { VTuber } from '../models';
import { getValidUserToken } from './twitch-user-token';
import { syncFromTwitch } from './sync';

function requireClientId(): string {
  const id = process.env.TWITCH_CLIENT_ID;
  if (!id) throw new Error('TWITCH_CLIENT_ID is not set in .env');
  return id;
}

interface EventSubState {
  socket: WebSocket | null;
  sessionId: string | null;
}

// `import.meta.hot` only exists under `bun --hot` (what `bun run dev` uses).
// Stashing the connection in hot.data means a file edit reuses the same
// socket instead of opening a new one on every save — Bun's own docs warn
// that using `dispose()` here would reopen the websocket on every hot
// update, so `prune()` is used instead: it only fires when this module is
// actually removed from the module graph, not on ordinary edits.
const hot = (import.meta as any).hot;
const state: EventSubState = hot
  ? (hot.data.twitchEventSub ??= { socket: null, sessionId: null })
  : { socket: null, sessionId: null };

if (hot) {
  hot.prune(() => {
    state.socket?.close();
    state.socket = null;
    state.sessionId = null;
  });
}

async function helixFetch(path: string, init: RequestInit = {}): Promise<Response> {
  // EventSub subscription management (create/list/delete) always requires
  // a user access token when using websocket transport, even though the
  // app access token (used everywhere else in sync.ts) works fine for
  // webhook transport or for the unrelated Helix calls in this project.
  const token = await getValidUserToken();
  return fetch(`https://api.twitch.tv/helix${path}`, {
    ...init,
    headers: {
      ...init.headers,
      Authorization: `Bearer ${token}`,
      'Client-Id': requireClientId(),
    },
  });
}

async function createSubscription(broadcasterId: string, sessionId: string): Promise<void> {
  const res = await helixFetch('/eventsub/subscriptions', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      type: 'stream.online',
      version: '1',
      condition: { broadcaster_user_id: broadcasterId },
      transport: { method: 'websocket', session_id: sessionId },
    }),
  });

  // 409 = a subscription for this broadcaster/type/transport already
  // exists (e.g. reconcile ran twice) — not an error, just a no-op.
  if (!res.ok && res.status !== 409) {
    console.error(`Failed to subscribe to stream.online for ${broadcasterId}: ${res.status} ${await res.text()}`);
  }
}

interface HelixSubscription {
  id: string;
  status: string;
  condition: { broadcaster_user_id?: string };
}

async function listSubscriptions(): Promise<HelixSubscription[]> {
  const res = await helixFetch('/eventsub/subscriptions?type=stream.online');
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

/**
 * Diff Twitch's actual subscription list against currently-tracked Twitch
 * VTubers: drop subscriptions for anyone no longer tracked, add
 * subscriptions for anyone tracked but missing one. Runs once per
 * `session_welcome` (fresh connect or reconnect), which also self-heals
 * anything that drifted while the backend was down.
 */
async function reconcileSubscriptions(sessionId: string): Promise<void> {
  const [trackedVtubers, existingSubs] = await Promise.all([
    VTuber.find({ platform: 'twitch', isTracked: true }),
    listSubscriptions(),
  ]);

  const trackedIds = new Set(trackedVtubers.map((v) => v.platformChannelId));

  // Drop anything not tracked anymore, and anything left over from a dead
  // connection: once a websocket session closes, Twitch flips its
  // subscriptions' status to e.g. "websocket_disconnected" rather than
  // deleting them — they never deliver events again, but they'd otherwise
  // be mistaken below for live coverage of a broadcaster and block a real
  // subscription from being created for the current session.
  await Promise.all(
    existingSubs
      .filter(
        (sub) =>
          sub.status !== 'enabled' ||
          !sub.condition.broadcaster_user_id ||
          !trackedIds.has(sub.condition.broadcaster_user_id)
      )
      .map((sub) => deleteSubscription(sub.id))
  );

  const liveSubscribedIds = new Set(
    existingSubs.filter((sub) => sub.status === 'enabled').map((sub) => sub.condition.broadcaster_user_id)
  );

  await Promise.all(
    trackedVtubers
      .filter((v) => !liveSubscribedIds.has(v.platformChannelId))
      .map((v) => createSubscription(v.platformChannelId, sessionId))
  );
}

async function handleNotification(payload: any): Promise<void> {
  if (payload.subscription?.type !== 'stream.online') return;

  const broadcasterId = payload.event?.broadcaster_user_id;
  if (!broadcasterId) return;

  const vtuber = await VTuber.findOne({ platform: 'twitch', platformChannelId: broadcasterId });
  if (!vtuber) return;

  console.log(`${vtuber.name} just went live on Twitch — syncing`);
  await syncFromTwitch(vtuber._id.toString(), true);
}

function connect(url = 'wss://eventsub.wss.twitch.tv/ws'): void {
  const previousSocket = state.socket;
  const ws = new WebSocket(url);
  state.socket = ws;

  ws.addEventListener('message', (event) => {
    const message = JSON.parse(event.data as string);

    switch (message.metadata.message_type) {
      case 'session_welcome':
        state.sessionId = message.payload.session.id;
        // Only close the old connection once the new one is confirmed live —
        // this is the documented reconnect handoff, not a fresh connect.
        if (previousSocket && previousSocket !== ws) {
          previousSocket.close();
        }
        reconcileSubscriptions(state.sessionId!).catch((err) =>
          console.error('EventSub subscription reconciliation failed:', err)
        );
        break;

      case 'session_keepalive':
        break;

      case 'notification':
        handleNotification(message.payload).catch((err) =>
          console.error('Failed to handle EventSub notification:', err)
        );
        break;

      case 'session_reconnect':
        // Twitch preserves subscriptions across this handoff automatically.
        connect(message.payload.session.reconnect_url);
        break;

      case 'revocation':
        console.warn('EventSub subscription revoked:', message.payload.subscription);
        break;
    }
  });

  ws.addEventListener('close', () => {
    if (state.socket === ws) {
      state.socket = null;
      state.sessionId = null;
    }
  });

  ws.addEventListener('error', (err) => {
    console.error('Twitch EventSub websocket error:', err);
  });
}

export function startEventSubListener(): void {
  if (!process.env.TWITCH_CLIENT_ID) {
    console.warn('TWITCH_CLIENT_ID not set — skipping Twitch EventSub listener');
    return;
  }
  if (state.socket) {
    // Connection survived a hot reload via import.meta.hot.data — reuse it.
    return;
  }
  connect();
}

export async function subscribeToLive(broadcasterId: string): Promise<void> {
  if (!state.sessionId) {
    // Narrow startup window before the first session_welcome arrives.
    // The next reconnect's reconcileSubscriptions() will pick this up.
    console.warn(`EventSub session not ready yet — ${broadcasterId} will be picked up on next reconcile`);
    return;
  }
  await createSubscription(broadcasterId, state.sessionId);
}

export async function unsubscribeFromLive(broadcasterId: string): Promise<void> {
  const existing = await listSubscriptions();
  const match = existing.find((sub) => sub.condition.broadcaster_user_id === broadcasterId);
  if (match) {
    await deleteSubscription(match.id);
  }
}
