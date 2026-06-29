import { IStreamInput, IStatSnapshotInput, IClipInput } from '../../models';

/**
 * Parses Twitch duration format (e.g. 4h44m10s, 1h2m, 45m, 30s) to seconds.
 */
function parseTwitchDuration(durationStr: string): number {
  if (!durationStr) return 0;
  let seconds = 0;
  const hoursMatch = durationStr.match(/(\d+)h/);
  const minutesMatch = durationStr.match(/(\d+)m/);
  const secondsMatch = durationStr.match(/(\d+)s/);

  if (hoursMatch) seconds += parseInt(hoursMatch[1], 10) * 3600;
  if (minutesMatch) seconds += parseInt(minutesMatch[1], 10) * 60;
  if (secondsMatch) seconds += parseInt(secondsMatch[1], 10);

  if (!hoursMatch && !minutesMatch && !secondsMatch) {
    const num = parseInt(durationStr, 10);
    if (!isNaN(num)) seconds = num;
  }
  return seconds;
}

export function mapTwitchLiveStream(liveStream: any, vtuberId: string, userLogin: string): IStreamInput {
  return {
    vtuberId,
    externalId: liveStream.id,
    title: liveStream.title,
    platform: 'twitch',
    startTime: new Date(liveStream.started_at),
    endTime: null,
    duration: null,
    status: 'live',
    url: `https://www.twitch.tv/${userLogin}`,
    thumbnailUrl: liveStream.thumbnail_url.replace('{width}', '640').replace('{height}', '360'),
    sourceApi: 'twitch_api',
  };
}

export function mapTwitchVod(video: any, vtuberId: string): IStreamInput {
  const duration = parseTwitchDuration(video.duration);
  const startTime = new Date(video.created_at);
  const endTime = new Date(startTime.getTime() + duration * 1000);

  return {
    vtuberId,
    externalId: video.id,
    title: video.title,
    platform: 'twitch',
    startTime,
    endTime,
    duration,
    status: 'ended',
    url: video.url,
    thumbnailUrl: video.thumbnail_url.replace('%{width}', '640').replace('%{height}', '360'),
    sourceApi: 'twitch_api',
  };
}

export function mapTwitchStatSnapshot(followers: number, vtuberId: string): IStatSnapshotInput {
  return {
    vtuberId,
    subscriberCount: followers,
    viewCount: 0,
    capturedAt: new Date(),
    sourceApi: 'twitch_api',
  };
}

export function mapTwitchClip(clip: any, vtuberId: string, sourceStreamId: string | null): IClipInput {
  return {
    vtuberId,
    sourceStreamId,
    externalId: clip.id,
    title: clip.title,
    url: clip.url,
    viewCount: clip.view_count || 0,
    createdAt: new Date(clip.created_at),
    sourceApi: 'twitch_api',
  };
}
