import { Hono } from 'hono';
import { holodexTestRoute } from './routes/holodex.test';
import { youtubeTestRoute } from './routes/youtube.test';
import { twitchTestRoute } from './routes/twitch.test';
import { vtubersRoute } from './routes/vtubers';
import { syncRoute } from './routes/sync';
import { eventsubRoute } from './routes/eventsub';
import { connectToDatabase } from './lib/db';
import { initTwitchEventSub } from './lib/twitch-eventsub';
import { startYoutubeLivePoller, startStatsSyncPoller } from './lib/scheduler';
import { requireApiToken, assertApiTokenConfigured } from './lib/require-api-token';

assertApiTokenConfigured();

const app = new Hono();

// Everything that touches the database or spends an external API quota sits
// behind the shared secret. /eventsub/* is deliberately NOT covered: Twitch
// calls that route directly and cannot carry our bearer token. Its safety
// rests on HMAC signature verification (see routes/eventsub.ts) instead —
// strictly better than the old /auth/* exemption, which only relied on the
// callback being useless without a Twitch-issued code.
app.use('/api/*', requireApiToken);
app.use('/sync/*', requireApiToken);

// Mount API routes
app.route('/', vtubersRoute);
app.route('/', syncRoute);
app.route('/', eventsubRoute);

// Mount test routes
app.route('/', holodexTestRoute);
app.route('/', youtubeTestRoute);
app.route('/', twitchTestRoute);

// Deliberately no `export default app`. Bun's entry wrapper checks whether
// the entry module's default export looks like a server config, and a Hono
// instance does (it has `fetch`) — so exporting it makes Bun call
// Bun.serve() on it *in addition* to the explicit call below, and the second
// bind fails with EADDRINUSE. `bun run --hot` skips that wrapper, so this
// only ever showed up outside dev (e.g. in the container).

try {
  await connectToDatabase();
} catch {
  // connectToDatabase() already logged the reason. Exiting here rather than
  // letting the rejection go unhandled keeps that one line as the whole
  // output — an unhandled rejection would print the entire Mongoose error
  // object again, on every restart of a restarting container.
  process.exit(1);
}
initTwitchEventSub();
startYoutubeLivePoller();
startStatsSyncPoller();

// Configurable so the container can be remapped without a rebuild; keeps
// the previous hardcoded 3000 as the default for local dev.
const port = Number(process.env.PORT) || 3000;

Bun.serve({
  port,
  // Bun defaults to 10s and measures *socket silence*, not request duration —
  // it can't tell a still-working handler from a vanished client. A forced
  // sync fans out to Twitch/HoloDex/YouTube and routinely runs past that, so
  // Bun closed the connection mid-request and Caddy reported it as 502.
  // 120s is well clear of the slowest observed sync (Bun caps this at 255).
  idleTimeout: 120,
  fetch: app.fetch
})

console.log(`Listening on port ${port}`);

