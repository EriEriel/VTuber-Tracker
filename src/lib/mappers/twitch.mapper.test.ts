import { describe, it, expect } from 'bun:test';
import { mapTwitchLiveStream, mapTwitchVod, mapTwitchStatSnapshot, mapTwitchClip } from './twitch.mapper';

const VTUBER_ID = 'vtuber-twitch-789';

describe('mapTwitchLiveStream', () => {
  const baseLiveStream = {
    id: 'twitch-stream-id',
    title: 'Live Now!',
    started_at: '2024-03-01T14:00:00.000Z',
    thumbnail_url: 'https://static-cdn.jtvnw.net/previews-ttv/live_user_test-{width}x{height}.jpg',
  };

  it('maps core fields', () => {
    const result = mapTwitchLiveStream(baseLiveStream, VTUBER_ID, 'teststreamer');
    expect(result.vtuberId).toBe(VTUBER_ID);
    expect(result.externalId).toBe('twitch-stream-id');
    expect(result.title).toBe('Live Now!');
    expect(result.platform).toBe('twitch');
    expect(result.sourceApi).toBe('twitch_api');
  });

  it('always sets status to "live", endTime and duration to null', () => {
    const result = mapTwitchLiveStream(baseLiveStream, VTUBER_ID, 'teststreamer');
    expect(result.status).toBe('live');
    expect(result.endTime).toBeNull();
    expect(result.duration).toBeNull();
  });

  it('builds URL from userLogin', () => {
    const result = mapTwitchLiveStream(baseLiveStream, VTUBER_ID, 'teststreamer');
    expect(result.url).toBe('https://www.twitch.tv/teststreamer');
  });

  it('replaces {width} and {height} in thumbnail_url', () => {
    const result = mapTwitchLiveStream(baseLiveStream, VTUBER_ID, 'teststreamer');
    expect(result.thumbnailUrl).toBe(
      'https://static-cdn.jtvnw.net/previews-ttv/live_user_test-640x360.jpg'
    );
  });

  it('sets startTime from started_at', () => {
    const result = mapTwitchLiveStream(baseLiveStream, VTUBER_ID, 'teststreamer');
    expect(result.startTime).toEqual(new Date('2024-03-01T14:00:00.000Z'));
  });
});

describe('mapTwitchVod', () => {
  const baseVod = {
    id: 'twitch-vod-id',
    title: 'Past Stream VOD',
    created_at: '2024-03-01T14:00:00.000Z',
    duration: '4h44m10s',
    url: 'https://www.twitch.tv/videos/12345',
    thumbnail_url: 'https://vod-secure.twitch.tv/id/%{width}x%{height}.jpg',
  };

  it('maps core fields', () => {
    const result = mapTwitchVod(baseVod, VTUBER_ID);
    expect(result.vtuberId).toBe(VTUBER_ID);
    expect(result.externalId).toBe('twitch-vod-id');
    expect(result.platform).toBe('twitch');
    expect(result.status).toBe('ended');
    expect(result.sourceApi).toBe('twitch_api');
    expect(result.url).toBe('https://www.twitch.tv/videos/12345');
  });

  it.each([
    ['4h44m10s', 4 * 3600 + 44 * 60 + 10],
    ['1h2m', 3720],
    ['45m', 2700],
    ['30s', 30],
    ['1h', 3600],
    ['', 0],
  ])('parses Twitch duration "%s" → %i seconds', (durationStr, expected) => {
    const result = mapTwitchVod({ ...baseVod, duration: durationStr }, VTUBER_ID);
    expect(result.duration).toBe(expected);
  });

  it('computes endTime as startTime + duration', () => {
    const result = mapTwitchVod(baseVod, VTUBER_ID);
    const expectedStart = new Date('2024-03-01T14:00:00.000Z');
    const expectedEnd = new Date(expectedStart.getTime() + 17050 * 1000);
    expect(result.startTime).toEqual(expectedStart);
    expect(result.endTime).toEqual(expectedEnd);
  });

  it('replaces %{width} and %{height} in thumbnail_url', () => {
    const result = mapTwitchVod(baseVod, VTUBER_ID);
    expect(result.thumbnailUrl).toBe('https://vod-secure.twitch.tv/id/640x360.jpg');
  });
});

describe('mapTwitchStatSnapshot', () => {
  it('sets subscriberCount from followers and viewCount to 0', () => {
    const result = mapTwitchStatSnapshot(75000, VTUBER_ID);
    expect(result.subscriberCount).toBe(75000);
    expect(result.viewCount).toBe(0);
    expect(result.sourceApi).toBe('twitch_api');
    expect(result.vtuberId).toBe(VTUBER_ID);
    expect(result.capturedAt).toBeInstanceOf(Date);
  });

  it('handles 0 followers', () => {
    const result = mapTwitchStatSnapshot(0, VTUBER_ID);
    expect(result.subscriberCount).toBe(0);
  });
});

describe('mapTwitchClip', () => {
  const baseClip = {
    id: 'twitch-clip-id',
    title: 'Epic Clip',
    url: 'https://clips.twitch.tv/epic-clip',
    view_count: 1500,
    created_at: '2024-03-05T16:30:00.000Z',
  };

  it('maps core fields', () => {
    const result = mapTwitchClip(baseClip, VTUBER_ID, 'stream-mongo-id');
    expect(result.vtuberId).toBe(VTUBER_ID);
    expect(result.externalId).toBe('twitch-clip-id');
    expect(result.title).toBe('Epic Clip');
    expect(result.url).toBe('https://clips.twitch.tv/epic-clip');
    expect(result.viewCount).toBe(1500);
    expect(result.sourceApi).toBe('twitch_api');
  });

  it('passes sourceStreamId through', () => {
    expect(mapTwitchClip(baseClip, VTUBER_ID, 'stream-mongo-id').sourceStreamId).toBe('stream-mongo-id');
  });

  it('passes null sourceStreamId when no linked stream', () => {
    expect(mapTwitchClip(baseClip, VTUBER_ID, null).sourceStreamId).toBeNull();
  });

  it('maps createdAt from created_at', () => {
    const result = mapTwitchClip(baseClip, VTUBER_ID, null);
    expect(result.createdAt).toEqual(new Date('2024-03-05T16:30:00.000Z'));
  });

  it('defaults viewCount to 0 when missing', () => {
    const result = mapTwitchClip({ ...baseClip, view_count: undefined }, VTUBER_ID, null);
    expect(result.viewCount).toBe(0);
  });
});
