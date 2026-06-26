import { getValidTwitchToken } from './lib/twitch-token';

async function testHolodex() {
  const apiKey = process.env.HOLODEX_API_KEY;
  if (!apiKey) throw new Error('No Holodex Key');

  console.log('--- HOLODEX CHANNEL ---');
  const channelRes = await fetch('https://holodex.net/api/v2/channels/UC1DCedRgGHBdm81E1llLhOQ', {
    headers: { 'X-APIKEY': apiKey }
  });
  const channelData = await channelRes.json();
  console.log('Channel keys:', Object.keys(channelData));
  console.log('Channel sample:', JSON.stringify({
    id: channelData.id,
    name: channelData.name,
    english_name: channelData.english_name,
    photo: channelData.photo,
    org: channelData.org,
    suborg: channelData.suborg,
    video_count: channelData.video_count,
    subscriber_count: channelData.subscriber_count,
    view_count: channelData.view_count,
  }, null, 2));

  console.log('--- HOLODEX VIDEOS ---');
  const videosRes = await fetch('https://holodex.net/api/v2/channels/UC1DCedRgGHBdm81E1llLhOQ/videos?limit=1', {
    headers: { 'X-APIKEY': apiKey }
  });
  const videosData = await videosRes.json() as any[];
  if (videosData.length > 0) {
    console.log('Video keys:', Object.keys(videosData[0]));
    console.log('Video sample:', JSON.stringify(videosData[0], null, 2));
  } else {
    console.log('No Holodex videos found.');
  }

  console.log('--- HOLODEX CLIPS ---');
  const clipsRes = await fetch('https://holodex.net/api/v2/channels/UC1DCedRgGHBdm81E1llLhOQ/clips?limit=1', {
    headers: { 'X-APIKEY': apiKey }
  });
  const clipsData = await clipsRes.json() as any[];
  if (clipsData.length > 0) {
    console.log('Clip keys:', Object.keys(clipsData[0]));
    console.log('Clip sample:', JSON.stringify(clipsData[0], null, 2));
  } else {
    console.log('No Holodex clips found.');
  }
}

async function testYoutube() {
  const apiKey = process.env.YOUTUBE_API_KEY;
  if (!apiKey) throw new Error('No Youtube Key');

  console.log('--- YOUTUBE CHANNEL ---');
  const res = await fetch(`https://www.googleapis.com/youtube/v3/channels?part=snippet,statistics&id=UC1DCedRgGHBdm81E1llLhOQ&key=${apiKey}`);
  const data = await res.json() as any;
  const channel = data.items?.[0];
  if (channel) {
    console.log('Channel keys:', Object.keys(channel));
    console.log('Channel snippet keys:', Object.keys(channel.snippet));
    console.log('Channel statistics keys:', Object.keys(channel.statistics));
    console.log('Channel sample:', JSON.stringify({
      id: channel.id,
      title: channel.snippet.title,
      thumbnails: channel.snippet.thumbnails,
      subscriberCount: channel.statistics.subscriberCount,
      viewCount: channel.statistics.viewCount,
    }, null, 2));
  }

  console.log('--- YOUTUBE VIDEOS (recent uploads playlist) ---');
  // First get the uploads playlist ID
  const uploadsPlaylistId = channel?.contentDetails?.relatedPlaylists?.uploads || `UU1DCedRgGHBdm81E1llLhOQ`; // UC -> UU
  const playlistRes = await fetch(`https://www.googleapis.com/youtube/v3/playlistItems?part=snippet&playlistId=${uploadsPlaylistId}&maxResults=1&key=${apiKey}`);
  const playlistData = await playlistRes.json() as any;
  if (playlistData.items?.[0]) {
    const item = playlistData.items[0];
    console.log('PlaylistItem keys:', Object.keys(item));
    console.log('PlaylistItem snippet keys:', Object.keys(item.snippet));
    console.log('PlaylistItem sample:', JSON.stringify(item.snippet, null, 2));

    // Get specific video details to retrieve status/duration
    const videoId = item.snippet.resourceId.videoId;
    const videoRes = await fetch(`https://www.googleapis.com/youtube/v3/videos?part=snippet,contentDetails,liveStreamingDetails&id=${videoId}&key=${apiKey}`);
    const videoData = await videoRes.json() as any;
    if (videoData.items?.[0]) {
      console.log('Video details sample:', JSON.stringify(videoData.items[0], null, 2));
    }
  }
}

async function testTwitch() {
  const clientId = process.env.TWITCH_CLIENT_ID;
  if (!clientId) throw new Error('No Twitch Client ID');

  const token = await getValidTwitchToken();
  console.log('Got twitch token:', token ? 'Success' : 'Fail');

  console.log('--- TWITCH USER ---');
  const userRes = await fetch('https://api.twitch.tv/helix/users?login=tawffie', {
    headers: { Authorization: `Bearer ${token}`, 'Client-Id': clientId }
  });
  const userData = await userRes.json() as any;
  const user = userData.data?.[0];
  if (user) {
    console.log('User sample:', JSON.stringify(user, null, 2));
    const userId = user.id;

    console.log('--- TWITCH VIDEOS ---');
    const videosRes = await fetch(`https://api.twitch.tv/helix/videos?user_id=${userId}&first=1`, {
      headers: { Authorization: `Bearer ${token}`, 'Client-Id': clientId }
    });
    const videosData = await videosRes.json() as any;
    if (videosData.data?.[0]) {
      console.log('Twitch Video sample:', JSON.stringify(videosData.data[0], null, 2));
    }

    console.log('--- TWITCH CLIPS ---');
    const clipsRes = await fetch(`https://api.twitch.tv/helix/clips?broadcaster_id=${userId}&first=1`, {
      headers: { Authorization: `Bearer ${token}`, 'Client-Id': clientId }
    });
    const clipsData = await clipsRes.json() as any;
    if (clipsData.data?.[0]) {
      console.log('Twitch Clip sample:', JSON.stringify(clipsData.data[0], null, 2));
    }

    console.log('--- TWITCH LIVE STREAMS ---');
    const streamsRes = await fetch(`https://api.twitch.tv/helix/streams?user_id=${userId}`, {
      headers: { Authorization: `Bearer ${token}`, 'Client-Id': clientId }
    });
    const streamsData = await streamsRes.json() as any;
    console.log('Twitch Live Stream sample:', JSON.stringify(streamsData.data?.[0] || 'No active stream', null, 2));
  }
}

async function main() {
  try {
    await testHolodex();
    await testYoutube();
    await testTwitch();
  } catch (err) {
    console.error('Error testing APIs:', err);
  }
}

main();
