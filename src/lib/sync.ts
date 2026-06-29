import { VTuber, Stream, Clip, StatSnapshot } from '../models';
import { getValidTwitchToken } from './twitch-token';
import { mapHolodexStream, mapHolodexStatSnapshot, mapHolodexClip } from './mappers/holodex.mappers';
import { mapYoutubeStream, mapYoutubeStatSnapshot } from './mappers/youtube.mapper';
import { mapTwitchLiveStream, mapTwitchVod, mapTwitchStatSnapshot, mapTwitchClip } from './mappers/twitch.mapper';

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
      let liveSyncSucceeded = false;
      let statsSyncSucceeded = false;

      if (!shouldSyncLive && !shouldSyncStats) {
        results.push({ vtuberId: vtuber._id, status: 'skipped', reason: 'data is fresh' });
        continue;
      }

      console.log(`Syncing ${vtuber.name} (HoloDex). Live: ${shouldSyncLive}, Stats: ${shouldSyncStats}`);

      // 1. Sync Live Status (Streams)
      if (shouldSyncLive) {
        const videosUrl = `https://holodex.net/api/v2/channels/${vtuber.platformChannelId}/videos?type=videos&limit=50`;
        try {
          const vRes = await fetch(videosUrl, { headers: { 'X-APIKEY': apiKey } });
          if (vRes.ok) {
            liveSyncSucceeded = true;
            const videos = await vRes.json() as any[];
            for (const video of videos) {
              try {
                await Stream.findOneAndUpdate(
                  { platform: 'youtube', externalId: video.id },
                  mapHolodexStream(video, vtuber._id.toString()),
                  { upsert: true, returnDocument: 'after' }
                );
              } catch (err) {
                console.error(`ID: ${video.id}, Database error:`, err);
              }
            }
          }
        } catch (err) {
          console.error(`Failed to fetch videos of ${vtuber.name} from Holodex API:`, err)
        }
      }

      // 2. Sync Channel Stats & Clips
      if (shouldSyncStats) {
        // Stats
        const channelUrl = `https://holodex.net/api/v2/channels/${vtuber.platformChannelId}`;
        try {
          const cRes = await fetch(channelUrl, { headers: { 'X-APIKEY': apiKey } });
          if (cRes.ok) {
            statsSyncSucceeded = true;
            const channel = await cRes.json() as any;

            // Update profile details if they changed
            if (channel.name) vtuber.name = channel.name;
            if (channel.english_name) vtuber.englishName = channel.english_name;
            if (channel.photo) vtuber.photo = channel.photo;
            if (channel.org) vtuber.org = channel.org;
            if (channel.suborg) vtuber.suborg = channel.suborg;

            try {
              await StatSnapshot.create(mapHolodexStatSnapshot(channel, vtuber._id.toString()));
            } catch (err) {
              console.error('Database operation failed:', err);
            }
          }
        } catch (err) {
          console.error(`Failed to fetch data from Holodex API:`, err)
        }

        // Clips
        const clipsUrl = `https://holodex.net/api/v2/channels/${vtuber.platformChannelId}/clips?limit=50`;
        try {
          const clRes = await fetch(clipsUrl, { headers: { 'X-APIKEY': apiKey } });
          if (clRes.ok) {
            const clips = await clRes.json() as any[];
            for (const clip of clips) {
              try {
                await Clip.findOneAndUpdate(
                  { sourceApi: 'holodex', externalId: clip.id },
                  mapHolodexClip(clip, vtuber._id.toString()),
                  { upsert: true, returnDocument: 'after' }
                );
              } catch (err) {
                console.error('Database operation failed:', err);
              }
            }
          }
        } catch (err) {
          console.error('Failed to fetch clips data from Holodex API:', err)
        }
      }

      // Update timestamps
      if (shouldSyncLive && liveSyncSucceeded) vtuber.lastLiveSyncedAt = now;
      if (shouldSyncStats && statsSyncSucceeded) vtuber.lastStatsSyncedAt = now;
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
      let liveSyncSucceeded = false;
      let statsSyncSucceeded = false;

      if (!shouldSyncLive && !shouldSyncStats) {
        results.push({ vtuberId: vtuber._id, status: 'skipped', reason: 'data is fresh' });
        continue;
      }

      console.log(`Syncing ${vtuber.name} (YouTube API). Live: ${shouldSyncLive}, Stats: ${shouldSyncStats}`);

      // 1. Sync Channel Stats
      if (shouldSyncStats) {
        try {
          const channelUrl = `https://www.googleapis.com/youtube/v3/channels?part=snippet,statistics&id=${vtuber.platformChannelId}&key=${apiKey}`;
          const cRes = await fetch(channelUrl);
          if (cRes.ok) {
            statsSyncSucceeded = true;
            const cData = await cRes.json() as any;
            const channel = cData.items?.[0];
            if (channel) {
              if (channel.snippet?.title) vtuber.name = channel.snippet.title;
              if (channel.snippet?.thumbnails?.high?.url) {
                vtuber.photo = channel.snippet.thumbnails.high.url;
              } else if (channel.snippet?.thumbnails?.default?.url) {
                vtuber.photo = channel.snippet.thumbnails.default.url;
              }

              await StatSnapshot.create(mapYoutubeStatSnapshot(channel, vtuber._id.toString()));
            }
          } else {
            console.error(`YouTube channel fetch non-OK for ${vtuber.name}: ${cRes.status}`);
          }
        } catch (err) {
          console.error(`Failed to fetch YouTube channel stats for ${vtuber.name}:`, err);
        }
      }

      // 2. Sync Streams (via Uploads Playlist + video details)
      if (shouldSyncLive) {
        try {
          // Map channel ID to uploads playlist ID by replacing 'UC' with 'UU'
          const uploadsPlaylistId = vtuber.platformChannelId.startsWith('UC')
            ? 'UU' + vtuber.platformChannelId.substring(2)
            : vtuber.platformChannelId;

          const playlistUrl = `https://www.googleapis.com/youtube/v3/playlistItems?part=snippet&playlistId=${uploadsPlaylistId}&maxResults=30&key=${apiKey}`;
          const plRes = await fetch(playlistUrl);
          if (plRes.ok) {
            liveSyncSucceeded = true;
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
                  await Stream.findOneAndUpdate(
                    { platform: 'youtube', externalId: video.id },
                    mapYoutubeStream(video, vtuber._id.toString()),
                    { upsert: true, returnDocument: 'after' }
                  );
                }
              } else {
                console.error(`YouTube videos fetch non-OK for ${vtuber.name}: ${vRes.status}`);
              }
            }
          } else {
            console.error(`YouTube playlist fetch non-OK for ${vtuber.name}: ${plRes.status}`);
          }
        } catch (err) {
          console.error(`Failed to sync live streams for ${vtuber.name} from YouTube API:`, err);
        }
      }

      // Update timestamps
      if (shouldSyncLive && liveSyncSucceeded) vtuber.lastLiveSyncedAt = now;
      if (shouldSyncStats && statsSyncSucceeded) vtuber.lastStatsSyncedAt = now;
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
      let liveSyncSucceeded = false;
      let statsSyncSucceeded = false;

      if (!shouldSyncLive && !shouldSyncStats) {
        results.push({ vtuberId: vtuber._id, status: 'skipped', reason: 'data is fresh' });
        continue;
      }

      console.log(`Syncing ${vtuber.name} (Twitch API). Live: ${shouldSyncLive}, Stats: ${shouldSyncStats}`);

      const token = await getValidTwitchToken();
      let userLogin = vtuber.englishName || vtuber.name;

      // 1. Sync User Profile details & Stats
      if (shouldSyncStats) {
        try {
          const user = await fetchTwitchUserById(vtuber.platformChannelId);
          if (user) {
            vtuber.name = user.display_name;
            vtuber.englishName = user.display_name;
            vtuber.photo = user.profile_image_url;
            userLogin = user.login;

            // Follower count as proxy for subscriberCount
            const followers = await fetchTwitchFollowerCount(vtuber.platformChannelId);

            await StatSnapshot.create(mapTwitchStatSnapshot(followers, vtuber._id.toString()));
            statsSyncSucceeded = true;
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
                mapTwitchClip(clip, vtuber._id.toString(), sourceStreamId ? sourceStreamId.toString() : null),
                { upsert: true, returnDocument: 'after' }
              );
            }
          } else {
            console.error(`Twitch clips fetch non-OK for ${vtuber.name}: ${clRes.status}`);
          }
        } catch (err) {
          console.error(`Failed to sync stats for ${vtuber.name} from Twitch API:`, err);
        }
      }

      // 2. Sync Streams (Live Stream & VODs)
      if (shouldSyncLive) {
        try {
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

          if (sRes.ok) {
            liveSyncSucceeded = true;
            const sData = await sRes.json() as any;
            const liveStream = sData.data?.[0];
            if (liveStream) {
              await Stream.findOneAndUpdate(
                { platform: 'twitch', externalId: liveStream.id },
                mapTwitchLiveStream(liveStream, vtuber._id.toString(), userLogin),
                { upsert: true, returnDocument: 'after' }
              );
            }
          } else {
            console.error(`Twitch streams fetch non-OK for ${vtuber.name}: ${sRes.status}`);
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
              await Stream.findOneAndUpdate(
                { platform: 'twitch', externalId: video.id },
                mapTwitchVod(video, vtuber._id.toString()),
                { upsert: true, returnDocument: 'after' }
              );
            }
          } else {
            console.error(`Twitch VODs fetch non-OK for ${vtuber.name}: ${vRes.status}`);
          }
        } catch (err) {
          console.error(`Failed to sync live streams for ${vtuber.name} from Twitch API:`, err);
        }
      }

      // Update timestamps
      if (shouldSyncLive && liveSyncSucceeded) vtuber.lastLiveSyncedAt = now;
      if (shouldSyncStats && statsSyncSucceeded) vtuber.lastStatsSyncedAt = now;
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
