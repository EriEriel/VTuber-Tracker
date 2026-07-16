import mongoose from 'mongoose';
import { connectToDatabase } from './lib/db';
import { VTuber, Stream, Clip, StatSnapshot } from './models';
import { syncFromHolodex, syncFromTwitch } from './lib/sync';

async function testResolutionAndSync() {
  console.log('Connecting to database...');
  await connectToDatabase();
  console.log('readyState before deleteMany:', mongoose.connection.readyState);

  // 1. Clear database to start fresh for test
  console.log('Clearing database collections...');
  await VTuber.deleteMany({});
  await Stream.deleteMany({});
  await Clip.deleteMany({});
  await StatSnapshot.deleteMany({});

  // Mocking Honos API request logic for registering a YouTube (HoloDex) channel
  console.log('\n--- Test 1: Registering YouTube (HoloDex) Talent (Pekora) ---');
  const pekoraChannelId = 'UC1DCedRgGHBdm81E1llLhOQ';

  // Call the resolution logic directly
  const apiKey = process.env.HOLODEX_API_KEY;
  let holodexSuccess = false;
  let name = '';
  let englishName = '';
  let photo = '';
  let source: 'holodex' | 'youtube_api' | 'twitch_api' = 'youtube_api';
  let org = '';
  let suborg = '';

  if (apiKey) {
    try {
      const res = await fetch(`https://holodex.net/api/v2/channels/${pekoraChannelId}`, {
        headers: { 'X-APIKEY': apiKey }
      });
      if (res.ok) {
        const data = await res.json() as any;
        name = data.name;
        englishName = data.english_name || name;
        photo = data.photo;
        org = data.org;
        suborg = data.suborg;
        source = 'holodex';
        holodexSuccess = true;
        console.log(`Successfully resolved from HoloDex: ${name} (${englishName}), Org: ${org}, Suborg: ${suborg}`);
      }
    } catch (e) {
      console.error(e);
    }
  }

  const vtuberPekora = await VTuber.create({
    name,
    englishName,
    photo,
    platform: 'youtube',
    source,
    platformChannelId: pekoraChannelId,
    org,
    suborg,
  });
  console.log('Pekora registered in DB:', vtuberPekora.toObject());

  // 2. Registering Twitch Talent
  console.log('\n--- Test 2: Registering Twitch Talent (tawffie) ---');
  const twitchLogin = 'tawffie';
  const { resolveTwitchUser } = require('./lib/sync');
  const twitchUser = await resolveTwitchUser(twitchLogin);
  if (!twitchUser) {
    throw new Error('Twitch user not resolved');
  }

  const vtuberTawffie = await VTuber.create({
    name: twitchUser.display_name,
    englishName: twitchUser.display_name,
    photo: twitchUser.profile_image_url,
    platform: 'twitch',
    source: 'twitch_api',
    platformChannelId: twitchUser.id,
  });
  console.log('Tawffie registered in DB:', vtuberTawffie.toObject());

  // 3. Sync from Holodex
  console.log('\n--- Test 3: Running syncFromHolodex (force=true) ---');
  const hdResults = await syncFromHolodex(vtuberPekora._id.toString(), true);
  console.log('Holodex Sync Results:', hdResults);

  // Check DB state
  const pkStreams = await Stream.find({ vtuberId: vtuberPekora._id });
  const pkClips = await Clip.find({ vtuberId: vtuberPekora._id });
  const pkSnapshots = await StatSnapshot.find({ vtuberId: vtuberPekora._id });
  console.log(`DB check Pekora: Streams=${pkStreams.length}, Clips=${pkClips.length}, Snapshots=${pkSnapshots.length}`);
  if (pkStreams.length > 0) {
    console.log('Pekora Stream sample:', pkStreams[0].toObject());
  }
  if (pkClips.length > 0) {
    console.log('Pekora Clip sample:', pkClips[0].toObject());
  }
  if (pkSnapshots.length > 0) {
    console.log('Pekora Snapshot sample:', pkSnapshots[0].toObject());
  }

  // 4. Sync from Twitch
  console.log('\n--- Test 4: Running syncFromTwitch (force=true) ---');
  const twResults = await syncFromTwitch(vtuberTawffie._id.toString(), true);
  console.log('Twitch Sync Results:', twResults);

  // Check DB state
  const twStreams = await Stream.find({ vtuberId: vtuberTawffie._id });
  const twClips = await Clip.find({ vtuberId: vtuberTawffie._id });
  const twSnapshots = await StatSnapshot.find({ vtuberId: vtuberTawffie._id });
  console.log(`DB check Tawffie: Streams=${twStreams.length}, Clips=${twClips.length}, Snapshots=${twSnapshots.length}`);
  if (twStreams.length > 0) {
    console.log('Tawffie Stream sample:', twStreams[0].toObject());
  }
  if (twClips.length > 0) {
    console.log('Tawffie Clip sample:', twClips[0].toObject());
  }
  if (twSnapshots.length > 0) {
    console.log('Tawffie Snapshot sample:', twSnapshots[0].toObject());
  }

  console.log('\nAll integration tests finished successfully!');
  process.exit(0);
}

testResolutionAndSync().catch(err => {
  console.error('Integration test failed:', err);
  process.exit(1);
});
