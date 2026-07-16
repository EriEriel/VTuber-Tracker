import { describe, it, expect, mock, beforeEach } from 'bun:test';
import { Hono } from 'hono';
import { syncRoute } from './sync';

// Bun hoists mock.module calls before static imports resolve, so these
// mock functions will be in place when syncRoute first imports '../lib/sync'.
const mockSyncFromHolodex = mock(async () => []);
const mockSyncFromYoutube = mock(async () => []);
const mockSyncFromTwitch = mock(async () => []);

mock.module('../lib/sync', () => ({
  syncFromHolodex: mockSyncFromHolodex,
  syncFromYoutube: mockSyncFromYoutube,
  syncFromTwitch: mockSyncFromTwitch,
}));

const app = new Hono();
app.route('/', syncRoute);

const SUCCESS_RESULT = [{ vtuberId: 'abc', status: 'success', synced: { live: true, stats: true } }];

beforeEach(() => {
  mockSyncFromHolodex.mockClear();
  mockSyncFromYoutube.mockClear();
  mockSyncFromTwitch.mockClear();
});

describe('POST /api/sync/holodex', () => {
  beforeEach(() => {
    mockSyncFromHolodex.mockResolvedValue(SUCCESS_RESULT);
  });

  it('returns source "holodex" with results', async () => {
    const res = await app.request('/api/sync/holodex', { method: 'POST' });
    expect(res.status).toBe(200);
    const body = await res.json() as any;
    expect(body.source).toBe('holodex');
    expect(body.results).toEqual(SUCCESS_RESULT);
  });

  it('passes ?id query param to syncFromHolodex', async () => {
    await app.request('/api/sync/holodex?id=my-vtuber-id', { method: 'POST' });
    expect(mockSyncFromHolodex).toHaveBeenCalledWith('my-vtuber-id', false);
  });

  it('passes force=true when ?force=true is set', async () => {
    await app.request('/api/sync/holodex?force=true', { method: 'POST' });
    expect(mockSyncFromHolodex).toHaveBeenCalledWith(undefined, true);
  });

  it('returns 500 with error detail when sync throws', async () => {
    mockSyncFromHolodex.mockRejectedValueOnce(new Error('API key invalid'));
    const res = await app.request('/api/sync/holodex', { method: 'POST' });
    expect(res.status).toBe(500);
    const body = await res.json() as any;
    expect(body.error).toBe('HoloDex sync failed');
    expect(body.detail).toContain('API key invalid');
  });
});

describe('POST /api/sync/youtube', () => {
  beforeEach(() => {
    mockSyncFromYoutube.mockResolvedValue(SUCCESS_RESULT);
  });

  it('returns source "youtube_api" with results', async () => {
    const res = await app.request('/api/sync/youtube', { method: 'POST' });
    expect(res.status).toBe(200);
    const body = await res.json() as any;
    expect(body.source).toBe('youtube_api');
    expect(body.results).toEqual(SUCCESS_RESULT);
  });

  it('passes ?id and ?force params to syncFromYoutube', async () => {
    await app.request('/api/sync/youtube?id=yt-id&force=true', { method: 'POST' });
    expect(mockSyncFromYoutube).toHaveBeenCalledWith('yt-id', true);
  });

  it('returns 500 when sync throws', async () => {
    mockSyncFromYoutube.mockRejectedValueOnce(new Error('Quota exceeded'));
    const res = await app.request('/api/sync/youtube', { method: 'POST' });
    expect(res.status).toBe(500);
    const body = await res.json() as any;
    expect(body.error).toBe('YouTube API sync failed');
  });
});

describe('POST /api/sync/twitch', () => {
  beforeEach(() => {
    mockSyncFromTwitch.mockResolvedValue(SUCCESS_RESULT);
  });

  it('returns source "twitch_api" with results', async () => {
    const res = await app.request('/api/sync/twitch', { method: 'POST' });
    expect(res.status).toBe(200);
    const body = await res.json() as any;
    expect(body.source).toBe('twitch_api');
    expect(body.results).toEqual(SUCCESS_RESULT);
  });

  it('returns 500 when sync throws', async () => {
    mockSyncFromTwitch.mockRejectedValueOnce(new Error('Token expired'));
    const res = await app.request('/api/sync/twitch', { method: 'POST' });
    expect(res.status).toBe(500);
    const body = await res.json() as any;
    expect(body.error).toBe('Twitch API sync failed');
  });
});

describe('POST /api/sync/all', () => {
  beforeEach(() => {
    mockSyncFromHolodex.mockResolvedValue([{ vtuberId: 'h1', status: 'success' }]);
    mockSyncFromYoutube.mockResolvedValue([
      { vtuberId: 'y1', status: 'success' },
      { vtuberId: 'y2', status: 'skipped' },
    ]);
    mockSyncFromTwitch.mockResolvedValue([{ vtuberId: 't1', status: 'success' }]);
  });

  it('returns a summary with ok:true and correct counts when all succeed', async () => {
    const res = await app.request('/api/sync/all', { method: 'POST' });
    expect(res.status).toBe(200);
    const body = await res.json() as any;
    expect(body.summary.holodex).toEqual(expect.objectContaining({ ok: true, count: 1 }));
    expect(body.summary.youtube).toEqual(expect.objectContaining({ ok: true, count: 2 }));
    expect(body.summary.twitch).toEqual(expect.objectContaining({ ok: true, count: 1 }));
  });

  it('passes force=true to all three sync functions', async () => {
    await app.request('/api/sync/all?force=true', { method: 'POST' });
    expect(mockSyncFromHolodex).toHaveBeenCalledWith(undefined, true);
    expect(mockSyncFromYoutube).toHaveBeenCalledWith(undefined, true);
    expect(mockSyncFromTwitch).toHaveBeenCalledWith(undefined, true);
  });

  it('returns HTTP 200 with ok:false for the failed source when one sync rejects', async () => {
    mockSyncFromHolodex.mockRejectedValueOnce(new Error('HoloDex down'));
    const res = await app.request('/api/sync/all', { method: 'POST' });
    expect(res.status).toBe(200);
    const body = await res.json() as any;
    expect(body.summary.holodex).toEqual(expect.objectContaining({ ok: false, error: expect.stringContaining('HoloDex down') }));
    expect(body.summary.youtube).toEqual(expect.objectContaining({ ok: true }));
    expect(body.summary.twitch).toEqual(expect.objectContaining({ ok: true }));
  });
});

