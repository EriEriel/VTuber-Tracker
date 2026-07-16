import { describe, it, expect } from 'bun:test';
import { mapHolodexStream, mapHolodexStatSnapshot, mapHolodexClip } from './holodex.mappers';

const VTUBER_ID = 'vtuber-holodex-abc';

describe('mapHolodexStream', () => {
  const baseVideo = {
    id: 'dQw4w9WgXcQ',
    title: 'Test Stream',
    status: 'live',
    available_at: '2024-01-15T18:00:00.000Z',
    duration: 3600,
  };

  it('maps core fields', () => {
    const result = mapHolodexStream(baseVideo, VTUBER_ID);
    expect(result.vtuberId).toBe(VTUBER_ID);
    expect(result.externalId).toBe('dQw4w9WgXcQ');
    expect(result.title).toBe('Test Stream');
    expect(result.platform).toBe('youtube');
    expect(result.sourceApi).toBe('holodex');
  });

  it('builds correct YouTube URL and thumbnail', () => {
    const result = mapHolodexStream(baseVideo, VTUBER_ID);
    expect(result.url).toBe('https://www.youtube.com/watch?v=dQw4w9WgXcQ');
    expect(result.thumbnailUrl).toBe('https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg');
  });

  it('computes endTime as startTime + duration', () => {
    const result = mapHolodexStream(baseVideo, VTUBER_ID);
    const expectedStart = new Date('2024-01-15T18:00:00.000Z');
    const expectedEnd = new Date(expectedStart.getTime() + 3600 * 1000);
    expect(result.startTime.getTime()).toBe(expectedStart.getTime());
    expect(result.endTime?.getTime()).toBe(expectedEnd.getTime());
  });

  it('sets endTime to null when duration is 0', () => {
    const result = mapHolodexStream({ ...baseVideo, duration: 0 }, VTUBER_ID);
    expect(result.endTime).toBeNull();
    expect(result.duration).toBe(0);
  });

  it('falls back to published_at when available_at is missing', () => {
    const video = { ...baseVideo, available_at: undefined, published_at: '2024-01-15T20:00:00.000Z' };
    const result = mapHolodexStream(video, VTUBER_ID);
    expect(result.startTime).toEqual(new Date('2024-01-15T20:00:00.000Z'));
  });

  it('maps status "past" → "ended"', () => {
    const result = mapHolodexStream({ ...baseVideo, status: 'past' }, VTUBER_ID);
    expect(result.status).toBe('ended');
  });

  it('maps status "upcoming" → "upcoming"', () => {
    const result = mapHolodexStream({ ...baseVideo, status: 'upcoming' }, VTUBER_ID);
    expect(result.status).toBe('upcoming');
  });

  it('maps status "live" → "live"', () => {
    const result = mapHolodexStream({ ...baseVideo, status: 'live' }, VTUBER_ID);
    expect(result.status).toBe('live');
  });

  it('maps unrecognized status → "unknown"', () => {
    const result = mapHolodexStream({ ...baseVideo, status: 'deleted' }, VTUBER_ID);
    expect(result.status).toBe('unknown');
  });
});

describe('mapHolodexStatSnapshot', () => {
  it('parses subscriber and view counts from strings', () => {
    const result = mapHolodexStatSnapshot(
      { subscriber_count: '1000000', view_count: '5000000' },
      VTUBER_ID
    );
    expect(result.subscriberCount).toBe(1000000);
    expect(result.viewCount).toBe(5000000);
    expect(result.sourceApi).toBe('holodex');
    expect(result.vtuberId).toBe(VTUBER_ID);
    expect(result.capturedAt).toBeInstanceOf(Date);
  });

  it('defaults to 0 for missing counts', () => {
    const result = mapHolodexStatSnapshot({}, VTUBER_ID);
    expect(result.subscriberCount).toBe(0);
    expect(result.viewCount).toBe(0);
  });
});

describe('mapHolodexClip', () => {
  const baseClip = {
    id: 'clip-yt-id',
    title: 'Clip Title',
    view_count: 500,
    published_at: '2024-01-10T12:00:00.000Z',
  };

  it('maps core fields', () => {
    const result = mapHolodexClip(baseClip, VTUBER_ID);
    expect(result.vtuberId).toBe(VTUBER_ID);
    expect(result.externalId).toBe('clip-yt-id');
    expect(result.title).toBe('Clip Title');
    expect(result.viewCount).toBe(500);
    expect(result.sourceApi).toBe('holodex');
    expect(result.sourceStreamId).toBeNull();
  });

  it('builds correct YouTube URL', () => {
    const result = mapHolodexClip(baseClip, VTUBER_ID);
    expect(result.url).toBe('https://www.youtube.com/watch?v=clip-yt-id');
  });

  it('uses published_at for createdAt', () => {
    const result = mapHolodexClip(baseClip, VTUBER_ID);
    expect(result.createdAt).toEqual(new Date('2024-01-10T12:00:00.000Z'));
  });

  it('falls back to available_at when published_at is missing', () => {
    const clip = { ...baseClip, published_at: undefined, available_at: '2024-01-11T08:00:00.000Z' };
    const result = mapHolodexClip(clip, VTUBER_ID);
    expect(result.createdAt).toEqual(new Date('2024-01-11T08:00:00.000Z'));
  });

  it('defaults viewCount to 0 when missing', () => {
    const result = mapHolodexClip({ ...baseClip, view_count: undefined }, VTUBER_ID);
    expect(result.viewCount).toBe(0);
  });
});
