import { Hono } from 'hono';
import { holodexTestRoute } from './routes/holodex.test';
import { youtubeTestRoute } from './routes/youtube.test';
import { twitchTestRoute } from './routes/twitch.test';
import { vtubersRoute } from './routes/vtubers';
import { syncRoute } from './routes/sync';
import { authRoute } from './routes/auth';
import { connectToDatabase } from './lib/db';
import { startEventSubListener } from './lib/twitch-eventsub';

const app = new Hono();

// Mount API routes
app.route('/', vtubersRoute);
app.route('/', syncRoute);
app.route('/', authRoute);

// Mount test routes
app.route('/', holodexTestRoute);
app.route('/', youtubeTestRoute);
app.route('/', twitchTestRoute);

export default app;

await connectToDatabase();
startEventSubListener();

Bun.serve({
  port: 3000,
  fetch: app.fetch
})

