import { IStreamInput, IStatSnapshotInput } from '../../models';

/**
 * Parses ISO 8601 duration format (e.g. PT55S, PT1H2M10S, PT2H40M) to seconds.
 */
function parseISO8601Duration(durationStr: string): number {
  if (!durationStr) return 0;
  const match = durationStr.match(/PT(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?/);
  if (!match) return 0;
  const hours = parseInt(match[1] || '0', 10);
  const minutes = parseInt(match[2] || '0', 10);
  const seconds = parseInt(match[3] || '0', 10);
  return hours * 3600 + minutes * 60 + seconds;
}

export function mapYoutubeStream(video: any, vtuberId: string): IStreamInput {
  const liveDetails = video.liveStreamingDetails;
  const contentDetails = video.contentDetails;

  const startTimeStr = liveDetails?.actualStartTime || liveDetails?.scheduledStartTime || video.snippet?.publishedAt;
  const startTime = startTimeStr ? new Date(startTimeStr) : new Date();

  const duration = contentDetails?.duration ? parseISO8601Duration(contentDetails.duration) : 0;

  let endTime = null;
  if (liveDetails?.actualEndTime) {
    endTime = new Date(liveDetails.actualEndTime);
  } else if (duration > 0 && !liveDetails) {
    endTime = new Date(startTime.getTime() + duration * 1000);
  }

  let status: 'upcoming' | 'live' | 'ended' | 'unknown' = 'ended';
  switch (video.snippet?.liveBroadcastContent) {
    case 'live': status = 'live'; break;
    case 'upcoming': status = 'upcoming'; break;
  }

  return {
    vtuberId,
    externalId: video.id,
    title: video.snippet?.title || 'Unknown Video',
    platform: 'youtube',
    startTime,
    endTime,
    duration,
    status,
    url: `https://www.youtube.com/watch?v=${video.id}`,
    thumbnailUrl: video.snippet?.thumbnails?.high?.url || video.snippet?.thumbnails?.default?.url || '',
    sourceApi: 'youtube_api',
  };
}

export function mapYoutubeStatSnapshot(channel: any, vtuberId: string): IStatSnapshotInput {
  return {
    vtuberId,
    subscriberCount: parseInt(channel.statistics?.subscriberCount || '0', 10),
    viewCount: parseInt(channel.statistics?.viewCount || '0', 10),
    capturedAt: new Date(),
    sourceApi: 'youtube_api',
  };
}
