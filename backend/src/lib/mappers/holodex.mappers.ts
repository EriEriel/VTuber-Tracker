import { IStreamInput, IStatSnapshotInput, IClipInput } from "../../models";

export function mapHolodexStream(video: any, vtuberId: string): IStreamInput {
  const startTime = new Date(video.available_at || video.published_at);
  const duration = video.duration || 0;

  const endTime =
    duration > 0
      ? new Date(startTime.getTime() + duration * 1000)
      : null

  let status: "upcoming" | "live" | "ended" | "unknown" = "unknown";

  switch (video.status) {
    case "upcoming":
      status = "upcoming";
      break;
    case "live":
      status = "live";
      break;
    case "past":
      status = "ended";
      break;
  }

  return {
    vtuberId,
    externalId: video.id,
    title: video.title,
    platform: 'youtube',
    startTime,
    endTime,
    duration,
    status,
    url: `https://www.youtube.com/watch?v=${video.id}`,
    thumbnailUrl: `https://i.ytimg.com/vi/${video.id}/hqdefault.jpg`,
    sourceApi: 'holodex',
  };
}

export function mapHolodexStatSnapshot(channel: any, vtuberId: string): IStatSnapshotInput {
  return {
    vtuberId,
    subscriberCount: parseInt(channel.subscriber_count || '0', 10),
    viewCount: parseInt(channel.view_count || '0', 10),
    capturedAt: new Date(),
    sourceApi: 'holodex',
  }
}

export function mapHolodexClip(clip: any, vtuberId: string): IClipInput {

  let sourceStreamId = null;
  // HoloDex clips don't explicitly link back to a single parent video ID in a standardized direct field in all listings,
  // but we can default it to null and let it be.

  return {
    vtuberId,
    sourceStreamId,
    externalId: clip.id,
    title: clip.title,
    url: `https://www.youtube.com/watch?v=${clip.id}`,
    viewCount: clip.view_count || 0,
    createdAt: new Date(clip.published_at || clip.available_at),
    sourceApi: 'holodex',
  }
}
