import { describe, it, expect } from 'bun:test';
import { mapYoutubeStream, mapYoutubeStatSnapshot } from './youtube.mapper';

const VTUBER_ID = 'vtuber-youtube-xyz';

describe('mapYoutubeStream', () => {
  const baseVideo = {
    id: 'yt-video-id',
    snippet: {
      title: 'My YouTube Stream',
      liveBroadcastContent: 'none',
      publishedAt: '2024-02-01T10:00:00.000Z',
      thumbnails: {
        high: { url: 'https://example.com/high.jpg' },
        default: { url: 'https://example.com/default.jpg' },
      },
    },
    contentDetails: { duration: 'PT1H30M' },
    liveStreamingDetails: {
      actualStartTime: '2024-02-01T10:00:00.000Z',
      actualEndTime: '2024-02-01T11:30:00.000Z',
    },
  };

  it('maps core fields', () => {
    const result = mapYoutubeStream(baseVideo, VTUBER_ID);
    expect(result.vtuberId).toBe(VTUBER_ID);
    expect(result.externalId).toBe('yt-video-id');
    expect(result.title).toBe('My YouTube Stream');
    expect(result.platform).toBe('youtube');
    expect(result.sourceApi).toBe('youtube_api');
    expect(result.url).toBe('https://www.youtube.com/watch?v=yt-video-id');
  });

  it('uses high thumbnail when available', () => {
    const result = mapYoutubeStream(baseVideo, VTUBER_ID);
    expect(result.thumbnailUrl).toBe('https://example.com/high.jpg');
  });

  it('falls back to default thumbnail when high is missing', () => {
    const video = {
      ...baseVideo,
      snippet: { ...baseVideo.snippet, thumbnails: { default: { url: 'https://example.com/default.jpg' } } },
    };
    const result = mapYoutubeStream(video, VTUBER_ID);
    expect(result.thumbnailUrl).toBe('https://example.com/default.jpg');
  });

  it('uses actualEndTime when present', () => {
    const result = mapYoutubeStream(baseVideo, VTUBER_ID);
    expect(result.endTime).toEqual(new Date('2024-02-01T11:30:00.000Z'));
  });

  it('computes endTime from startTime + duration when no actualEndTime and no liveStreamingDetails', () => {
    const video = { ...baseVideo, contentDetails: { duration: 'PT30M' }, liveStreamingDetails: null };
    const result = mapYoutubeStream(video, VTUBER_ID);
    const expectedStart = new Date('2024-02-01T10:00:00.000Z');
    expect(result.endTime?.getTime()).toBe(expectedStart.getTime() + 30 * 60 * 1000);
  });

  it.each([
    ['PT55S', 55],
    ['PT1H2M10S', 3730],
    ['PT2H40M', 9600],
    ['PT1H', 3600],
    ['PT5M', 300],
  ])('parses ISO 8601 duration %s → %i seconds', (durationStr, expected) => {
    const video = { ...baseVideo, contentDetails: { duration: durationStr }, liveStreamingDetails: null };
    const result = mapYoutubeStream(video, VTUBER_ID);
    expect(result.duration).toBe(expected);
  });

  it('maps liveBroadcastContent "live" → "live"', () => {
    const video = { ...baseVideo, snippet: { ...baseVideo.snippet, liveBroadcastContent: 'live' } };
    expect(mapYoutubeStream(video, VTUBER_ID).status).toBe('live');
  });

  it('maps liveBroadcastContent "upcoming" → "upcoming"', () => {
    const video = { ...baseVideo, snippet: { ...baseVideo.snippet, liveBroadcastContent: 'upcoming' } };
    expect(mapYoutubeStream(video, VTUBER_ID).status).toBe('upcoming');
  });

  it('defaults status to "ended" for non-live content', () => {
    expect(mapYoutubeStream(baseVideo, VTUBER_ID).status).toBe('ended');
  });

  it('defaults title to "Unknown Video" when snippet is null', () => {
    const video = { id: 'x', snippet: null, contentDetails: null, liveStreamingDetails: null };
    expect(mapYoutubeStream(video, VTUBER_ID).title).toBe('Unknown Video');
  });
});

describe('mapYoutubeStatSnapshot', () => {
  it('parses subscriber and view counts from strings', () => {
    const result = mapYoutubeStatSnapshot(
      { statistics: { subscriberCount: '2500000', viewCount: '80000000' } },
      VTUBER_ID
    );
    expect(result.subscriberCount).toBe(2500000);
    expect(result.viewCount).toBe(80000000);
    expect(result.sourceApi).toBe('youtube_api');
    expect(result.vtuberId).toBe(VTUBER_ID);
    expect(result.capturedAt).toBeInstanceOf(Date);
  });

  it('defaults to 0 when statistics are missing', () => {
    const result = mapYoutubeStatSnapshot({}, VTUBER_ID);
    expect(result.subscriberCount).toBe(0);
    expect(result.viewCount).toBe(0);
  });
});
