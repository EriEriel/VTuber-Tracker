// Shared-secret auth for the API.
//
// Every route here mutates a database or spends a rate-limited third-party
// quota (`POST /api/sync/all` can drain the 10k units/day YouTube budget,
// `DELETE /api/vtubers/:id` destroys data), so once the backend is reachable
// from anything but localhost it needs a gate in front of it.
//
// Deliberately a static shared secret rather than real user accounts: there
// is exactly one user, and sessions/passwords/refresh flows would be a lot
// of surface area to secure for no gain.

import type { MiddlewareHandler } from 'hono';
import { createHash, timingSafeEqual } from 'node:crypto';

// Hash both sides before comparing: timingSafeEqual throws outright on a
// length mismatch, which would itself leak how long the real token is, and
// hashing normalizes everything to 32 bytes.
function secretsMatch(provided: string, expected: string): boolean {
  const a = createHash('sha256').update(provided).digest();
  const b = createHash('sha256').update(expected).digest();
  return timingSafeEqual(a, b);
}

export const requireApiToken: MiddlewareHandler = async (c, next) => {
  const expected = process.env.API_TOKEN;

  // Unset means local development — see assertApiTokenConfigured(), which
  // stops the process from ever starting this way in production.
  if (!expected) return next();

  const header = c.req.header('Authorization') ?? '';
  const provided = header.startsWith('Bearer ') ? header.slice(7) : '';

  if (!provided || !secretsMatch(provided, expected)) {
    return c.json({ error: 'Unauthorized' }, 401);
  }

  return next();
};

/**
 * Fail closed at startup rather than silently serving an open API.
 *
 * The dangerous case is deploying with API_TOKEN accidentally unset: the
 * middleware above would wave every request through and nothing would look
 * wrong until someone found the port. NODE_ENV is set to production in the
 * Dockerfile, so a container without a token refuses to boot, while local
 * `bun run dev` keeps working with no configuration.
 */
export function assertApiTokenConfigured(): void {
  if (process.env.NODE_ENV === 'production' && !process.env.API_TOKEN) {
    console.error(
      'Refusing to start: NODE_ENV=production but API_TOKEN is not set, ' +
        'which would expose every endpoint unauthenticated.'
    );
    process.exit(1);
  }

  if (!process.env.API_TOKEN) {
    console.warn('API_TOKEN not set — API is unauthenticated (fine for localhost, not for a public host)');
  }
}
