import { connectToDatabase } from './db';
import { VTuber, Stream, Clip, StatSnapshot } from '../models';
import { getValidTwitchToken } from './twitch-token';

// Connect to DB automatically when importing sync functions (optional, but safe)
connectToDatabase();

/**
 * Parses ISO 8601 duration format (e.g. PT55S, PT1H2M10S, PT2H40M) to seconds.
 */
export function parseISO8601Duration(durationStr: string): number {
  if (!durationStr) return 0;
  const match = durationStr.match(/PT(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?/);
  if (!match) return 0;
  const hours = parseInt(match[1] || '0', 10);
  const minutes = parseInt(match[2] || '0', 10);
  const seconds = parseInt(match[3] || '0', 10);
  return hours * 3600 + minutes * 60 + seconds;
}

/**
 * Parses Twitch duration format (e.g. 4h44m10s, 1h2m, 45m, 30s) to seconds.
 */
export function parseTwitchDuration(durationStr: string): number {
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

/**
 * Resolve a Twitch login name to user details from the Twitch API.
 */
export async function resolveTwitchUser(login: string): Promise<{ id: string; display_name: string; profile_image_url: string; login: string } | null> {
  const clientId = process.env.TWITCH_CLIENT_ID;
  if (!clientId) {
    throw new Error('TWITCH_CLIENT_ID is not set in .env');
  }

  const token = await getValidTwitchToken();
  const url = `https://api.twitch.tv/helix/users?login=${encodeURIComponent(login)}`;

  const res = await fetch(url, {
    headers: {
      Authorization: `Bearer ${token}`,
      'Client-Id': clientId,
    },
  });

  if (!res.ok) {
    const body = await res.text();
    throw new Error(`Twitch resolved user failed: ${res.status} ${body}`);
  }

  const data = await res.json() as { data: Array<{ id: string; display_name: string; profile_image_url: string; login: string }> };
  return data.data?.[0] || null;
}

/**
 * Fetch Twitch user details by numeric ID.
 */
export async function fetchTwitchUserById(id: string): Promise<{ id: string; display_name: string; profile_image_url: string; login: string } | null> {
  const clientId = process.env.TWITCH_CLIENT_ID;
  if (!clientId) {
    throw new Error('TWITCH_CLIENT_ID is not set in .env');
  }

  const token = await getValidTwitchToken();
  const url = `https://api.twitch.tv/helix/users?id=${encodeURIComponent(id)}`;

  const res = await fetch(url, {
    headers: {
      Authorization: `Bearer ${token}`,
      'Client-Id': clientId,
    },
  });

  if (!res.ok) {
    return null;
  }

  const data = await res.json() as { data: Array<{ id: string; display_name: string; profile_image_url: string; login: string }> };
  return data.data?.[0] || null;
}

/**
 * Safe fetch for Twitch follower count (returns 0 if token lacks permissions or endpoint fails).
 */
export async function fetchTwitchFollowerCount(broadcasterId: string): Promise<number> {
  const clientId = process.env.TWITCH_CLIENT_ID;
  if (!clientId) return 0;

  try {
    const token = await getValidTwitchToken();
    const url = `https://api.twitch.tv/helix/channels/followers?broadcaster_id=${broadcasterId}`;
    const res = await fetch(url, {
      headers: {
        Authorization: `Bearer ${token}`,
        'Client-Id': clientId,
      },
    });

    if (!res.ok) return 0;
    const data = await res.json() as { total: number };
    return data.total || 0;
  } catch (err) {
    console.error('Failed to fetch Twitch follower count:', err);
    return 0;
  }
}

/**
 * Sync VTubers configured with HoloDex source.
 * Enforces staleness checks.
 */
export async function syncFromHolodex(vtuberId?: string, force = false): Promise<any[]> {
  const apiKey = process.env.HOLODEX_API_KEY;
  if (!apiKey) {
    throw new Error('HOLODEX_API_KEY is not set in .env');
  }

  const query: any = { source: 'holodex', isTracked: true };
  if (vtuberId) {
    query._id = vtuberId;
  }

  const vtubers = await VTuber.find(query);
  const results = [];

  for (const vtuber of vtubers) {
    try {
      const now = new Date();
      const shouldSyncLive = force || !vtuber.lastLiveSyncedAt || (now.getTime() - vtuber.lastLiveSyncedAt.getTime() > 15 * 60 * 1000);
      const shouldSyncStats = force || !vtuber.lastStatsSyncedAt || (now.getTime() - vtuber.lastStatsSyncedAt.getTime() > 24 * 60 * 60 * 1000);

      if (!shouldSyncLive && !shouldSyncStats) {
        results.push({ vtuberId: vtuber._id, status: 'skipped', reason: 'data is fresh' });
        continue;
      }

      console.log(`Syncing ${vtuber.name} (HoloDex). Live: ${shouldSyncLive}, Stats: ${shouldSyncStats}`);

      // 1. Sync Live Status (Streams)
      if (shouldSyncLive) {
        const videosUrl = `https://holodex.net/api/v2/channels/${vtuber.platformChannelId}/videos?type=videos&limit=50`;
        const vRes = await fetch(videosUrl, { headers: { 'X-APIKEY': apiKey } });
        if (vRes.ok) {
          const videos = await vRes.json() as any[];
          for (const video of videos) {
            // Map Holodex video to Stream model
            const startTime = new Date(video.available_at || video.published_at);
            const duration = video.duration || 0;
            const endTime = duration > 0 ? new Date(startTime.getTime() + duration * 1000) : null;

            let status = 'unknown';
            if (video.status === 'upcoming') status = 'upcoming';
            else if (video.status === 'live') status = 'live';
            else if (video.status === 'past') status = 'ended';

            await Stream.findOneAndUpdate(
              { platform: 'youtube', externalId: video.id },
              {
                vtuberId: vtuber._id,
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
              },
              { upsert: true, new: true }
            );
          }
        }
      }

      // 2. Sync Channel Stats & Clips
      if (shouldSyncStats) {
        // Stats
        const channelUrl = `https://holodex.net/api/v2/channels/${vtuber.platformChannelId}`;
        const cRes = await fetch(channelUrl, { headers: { 'X-APIKEY': apiKey } });
        if (cRes.ok) {
          const channel = await cRes.json() as any;

          // Update profile details if they changed
          if (channel.name) vtuber.name = channel.name;
          if (channel.english_name) vtuber.englishName = channel.english_name;
          if (channel.photo) vtuber.photo = channel.photo;
          if (channel.org) vtuber.org = channel.org;
          if (channel.suborg) vtuber.suborg = channel.suborg;

          // Record time-series snapshot
          await StatSnapshot.create({
            vtuberId: vtuber._id,
            subscriberCount: parseInt(channel.subscriber_count || '0', 10),
            viewCount: parseInt(channel.view_count || '0', 10),
            capturedAt: new Date(),
            sourceApi: 'holodex',
          });
        }

        // Clips
        const clipsUrl = `https://holodex.net/api/v2/channels/${vtuber.platformChannelId}/clips?limit=50`;
        const clRes = await fetch(clipsUrl, { headers: { 'X-APIKEY': apiKey } });
        if (clRes.ok) {
          const clips = await clRes.json() as any[];
          for (const clip of clips) {
            // Find parent stream if possible
            let sourceStreamId = null;
            // HoloDex clips don't explicitly link back to a single parent video ID in a standardized direct field in all listings,
            // but we can default it to null and let it be.

            await Clip.findOneAndUpdate(
              { sourceApi: 'holodex', externalId: clip.id },
              {
                vtuberId: vtuber._id,
                sourceStreamId,
                externalId: clip.id,
                title: clip.title,
                url: `https://www.youtube.com/watch?v=${clip.id}`,
                viewCount: clip.view_count || 0,
                createdAt: new Date(clip.published_at || clip.available_at),
                sourceApi: 'holodex',
              },
              { upsert: true, new: true }
            );
          }
        }
      }

      // Update timestamps
      if (shouldSyncLive) vtuber.lastLiveSyncedAt = now;
      if (shouldSyncStats) vtuber.lastStatsSyncedAt = now;
      vtuber.lastSyncedAt = now;
      await vtuber.save();

      results.push({ vtuberId: vtuber._id, status: 'success', synced: { live: shouldSyncLive, stats: shouldSyncStats } });
    } catch (err) {
      console.error(`Error syncing VTuber ${vtuber.name} from Holodex:`, err);
      results.push({ vtuberId: vtuber._id, status: 'failed', error: String(err) });
    }
  }

  return results;
}

/**
 * Sync VTubers configured with youtube_api source.
 * Enforces staleness checks.
 */
export async function syncFromYoutube(vtuberId?: string, force = false): Promise<any[]> {
  const apiKey = process.env.YOUTUBE_API_KEY;
  if (!apiKey) {
    throw new Error('YOUTUBE_API_KEY is not set in .env');
  }

  const query: any = { source: 'youtube_api', isTracked: true };
  if (vtuberId) {
    query._id = vtuberId;
  }

  const vtubers = await VTuber.find(query);
  const results = [];

  for (const vtuber of vtubers) {
    try {
      const now = new Date();
      const shouldSyncLive = force || !vtuber.lastLiveSyncedAt || (now.getTime() - vtuber.lastLiveSyncedAt.getTime() > 15 * 60 * 1000);
      const shouldSyncStats = force || !vtuber.lastStatsSyncedAt || (now.getTime() - vtuber.lastStatsSyncedAt.getTime() > 24 * 60 * 60 * 1000);

      if (!shouldSyncLive && !shouldSyncStats) {
        results.push({ vtuberId: vtuber._id, status: 'skipped', reason: 'data is fresh' });
        continue;
      }

      console.log(`Syncing ${vtuber.name} (YouTube API). Live: ${shouldSyncLive}, Stats: ${shouldSyncStats}`);

      // 1. Sync Channel Stats
      if (shouldSyncStats) {
        const channelUrl = `https://www.googleapis.com/youtube/v3/channels?part=snippet,statistics&id=${vtuber.platformChannelId}&key=${apiKey}`;
        const cRes = await fetch(channelUrl);
        if (cRes.ok) {
          const cData = await cRes.json() as any;
          const channel = cData.items?.[0];
          if (channel) {
            if (channel.snippet?.title) vtuber.name = channel.snippet.title;
            if (channel.snippet?.thumbnails?.high?.url) {
              vtuber.photo = channel.snippet.thumbnails.high.url;
            } else if (channel.snippet?.thumbnails?.default?.url) {
              vtuber.photo = channel.snippet.thumbnails.default.url;
            }

            // Record snapshot
            await StatSnapshot.create({
              vtuberId: vtuber._id,
              subscriberCount: parseInt(channel.statistics?.subscriberCount || '0', 10),
              viewCount: parseInt(channel.statistics?.viewCount || '0', 10),
              capturedAt: new Date(),
              sourceApi: 'youtube_api',
            });
          }
        }
      }

      // 2. Sync Streams (via Uploads Playlist + video details)
      if (shouldSyncLive) {
        // Map channel ID to uploads playlist ID by replacing 'UC' with 'UU'
        const uploadsPlaylistId = vtuber.platformChannelId.startsWith('UC')
          ? 'UU' + vtuber.platformChannelId.substring(2)
          : vtuber.platformChannelId;

        const playlistUrl = `https://www.googleapis.com/youtube/v3/playlistItems?part=snippet&playlistId=${uploadsPlaylistId}&maxResults=30&key=${apiKey}`;
        const plRes = await fetch(playlistUrl);
        if (plRes.ok) {
          const plData = await plRes.json() as any;
          const items = plData.items || [];
          const videoIds = items.map((item: any) => item.snippet?.resourceId?.videoId).filter(Boolean);

          if (videoIds.length > 0) {
            // Fetch detailed video info to check liveBroadcastContent and duration
            const videosUrl = `https://www.googleapis.com/youtube/v3/videos?part=snippet,contentDetails,liveStreamingDetails&id=${videoIds.join(',')}&key=${apiKey}`;
            const vRes = await fetch(videosUrl);
            if (vRes.ok) {
              const vData = await vRes.json() as any;
              const videos = vData.items || [];

              for (const video of videos) {
                const liveDetails = video.liveStreamingDetails;
                const contentDetails = video.contentDetails;

                const startTimeStr = liveDetails?.actualStartTime || liveDetails?.scheduledStartTime || video.snippet?.publishedAt;
                const startTime = startTimeStr ? new Date(startTimeStr) : new Date();

                const duration = contentDetails?.duration ? parseISO8601Duration(contentDetails.duration) : 0;

                let endTime = null;
                if (liveDetails?.actualEndTime) {
                  endTime = new Date(liveDetails.actualEndTime);
                } else if (duration > 0 && !liveDetails) {
                  // Standard video uploaded
                  endTime = new Date(startTime.getTime() + duration * 1000);
                }

                let status = 'ended';
                const broadcastContent = video.snippet?.liveBroadcastContent;
                if (broadcastContent === 'live') status = 'live';
                else if (broadcastContent === 'upcoming') status = 'upcoming';

                await Stream.findOneAndUpdate(
                  { platform: 'youtube', externalId: video.id },
                  {
                    vtuberId: vtuber._id,
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
                  },
                  { upsert: true, new: true }
                );
              }
            }
          }
        }
      }

      // Update timestamps
      if (shouldSyncLive) vtuber.lastLiveSyncedAt = now;
      if (shouldSyncStats) vtuber.lastStatsSyncedAt = now;
      vtuber.lastSyncedAt = now;
      await vtuber.save();

      results.push({ vtuberId: vtuber._id, status: 'success', synced: { live: shouldSyncLive, stats: shouldSyncStats } });
    } catch (err) {
      console.error(`Error syncing VTuber ${vtuber.name} from YouTube API:`, err);
      results.push({ vtuberId: vtuber._id, status: 'failed', error: String(err) });
    }
  }

  return results;
}

/**
 * Sync VTubers configured with twitch_api source.
 * Enforces staleness checks.
 */
export async function syncFromTwitch(vtuberId?: string, force = false): Promise<any[]> {
  const clientId = process.env.TWITCH_CLIENT_ID;
  if (!clientId) {
    throw new Error('TWITCH_CLIENT_ID is not set in .env');
  }

  const query: any = { source: 'twitch_api', isTracked: true };
  if (vtuberId) {
    query._id = vtuberId;
  }

  const vtubers = await VTuber.find(query);
  const results = [];

  for (const vtuber of vtubers) {
    try {
      const now = new Date();
      const shouldSyncLive = force || !vtuber.lastLiveSyncedAt || (now.getTime() - vtuber.lastLiveSyncedAt.getTime() > 15 * 60 * 1000);
      const shouldSyncStats = force || !vtuber.lastStatsSyncedAt || (now.getTime() - vtuber.lastStatsSyncedAt.getTime() > 24 * 60 * 60 * 1000);

      if (!shouldSyncLive && !shouldSyncStats) {
        results.push({ vtuberId: vtuber._id, status: 'skipped', reason: 'data is fresh' });
        continue;
      }

      console.log(`Syncing ${vtuber.name} (Twitch API). Live: ${shouldSyncLive}, Stats: ${shouldSyncStats}`);

      const token = await getValidTwitchToken();
      let userLogin = vtuber.englishName || vtuber.name;

      // 1. Sync User Profile details & Stats
      if (shouldSyncStats) {
        const user = await fetchTwitchUserById(vtuber.platformChannelId);
        if (user) {
          vtuber.name = user.display_name;
          vtuber.englishName = user.display_name;
          vtuber.photo = user.profile_image_url;
          userLogin = user.login;

          // Follower count as proxy for subscriberCount
          const followers = await fetchTwitchFollowerCount(vtuber.platformChannelId);

          await StatSnapshot.create({
            vtuberId: vtuber._id,
            subscriberCount: followers,
            viewCount: 0, // Twitch Helix users endpoint no longer provides aggregate views
            capturedAt: new Date(),
            sourceApi: 'twitch_api',
          });
        }

        // Sync Clips
        const clipsUrl = `https://api.twitch.tv/helix/clips?broadcaster_id=${vtuber.platformChannelId}&first=50`;
        const clRes = await fetch(clipsUrl, {
          headers: {
            Authorization: `Bearer ${token}`,
            'Client-Id': clientId,
          },
        });

        if (clRes.ok) {
          const clData = await clRes.json() as any;
          const clips = clData.data || [];

          for (const clip of clips) {
            // Try to find the local Twitch Stream mapping this clip's video_id
            let sourceStreamId = null;
            if (clip.video_id) {
              const streamDoc = await Stream.findOne({ platform: 'twitch', externalId: clip.video_id });
              if (streamDoc) {
                sourceStreamId = streamDoc._id;
              }
            }

            await Clip.findOneAndUpdate(
              { sourceApi: 'twitch_api', externalId: clip.id },
              {
                vtuberId: vtuber._id,
                sourceStreamId,
                externalId: clip.id,
                title: clip.title,
                url: clip.url,
                viewCount: clip.view_count || 0,
                createdAt: new Date(clip.created_at),
                sourceApi: 'twitch_api',
              },
              { upsert: true, new: true }
            );
          }
        }
      }

      // 2. Sync Streams (Live Stream & VODs)
      if (shouldSyncLive) {
        // Get user login just in case
        if (!userLogin || userLogin === vtuber.name) {
          const user = await fetchTwitchUserById(vtuber.platformChannelId);
          if (user) {
            userLogin = user.login;
          }
        }

        // Fetch Live Stream
        const streamsUrl = `https://api.twitch.tv/helix/streams?user_id=${vtuber.platformChannelId}`;
        const sRes = await fetch(streamsUrl, {
          headers: {
            Authorization: `Bearer ${token}`,
            'Client-Id': clientId,
          },
        });

        let liveStreamId = null;
        if (sRes.ok) {
          const sData = await sRes.json() as any;
          const liveStream = sData.data?.[0];
          if (liveStream) {
            liveStreamId = liveStream.id;

            await Stream.findOneAndUpdate(
              { platform: 'twitch', externalId: liveStream.id },
              {
                vtuberId: vtuber._id,
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
              },
              { upsert: true, new: true }
            );
          }
        }

        // Fetch Recent VODs
        const videosUrl = `https://api.twitch.tv/helix/videos?user_id=${vtuber.platformChannelId}&first=20&type=archive`;
        const vRes = await fetch(videosUrl, {
          headers: {
            Authorization: `Bearer ${token}`,
            'Client-Id': clientId,
          },
        });

        if (vRes.ok) {
          const vData = await vRes.json() as any;
          const videos = vData.data || [];

          for (const video of videos) {
            // If this VOD is current live stream, ignore or map accordingly
            // (Normally live stream and VOD have different IDs, Twitch generates VOD shortly after live starts)
            const duration = parseTwitchDuration(video.duration);
            const startTime = new Date(video.created_at);
            const endTime = new Date(startTime.getTime() + duration * 1000);

            await Stream.findOneAndUpdate(
              { platform: 'twitch', externalId: video.id },
              {
                vtuberId: vtuber._id,
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
              },
              { upsert: true, new: true }
            );
          }
        }
      }

      // Update timestamps
      if (shouldSyncLive) vtuber.lastLiveSyncedAt = now;
      if (shouldSyncStats) vtuber.lastStatsSyncedAt = now;
      vtuber.lastSyncedAt = now;
      await vtuber.save();

      results.push({ vtuberId: vtuber._id, status: 'success', synced: { live: shouldSyncLive, stats: shouldSyncStats } });
    } catch (err) {
      console.error(`Error syncing VTuber ${vtuber.name} from Twitch API:`, err);
      results.push({ vtuberId: vtuber._id, status: 'failed', error: String(err) });
    }
  }

  return results;
}
