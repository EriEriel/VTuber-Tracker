// The only place in the codebase that writes `status: 'live'` or transitions
// a stream to `'ended'`. Centralized so Twitch's EventSub-triggered writes
// and the planned YouTube poll-triggered writes can't diverge into two
// implementations of the most correctness-sensitive write in the system.
// See LIVE_DETECTION.md (Phase 0) for the full rationale.

import mongoose from 'mongoose';
import { Stream, IStreamInput } from '../models';

/**
 * Mark a VTuber as live on a specific stream.
 *
 * Closes any *other* stream currently marked live for this VTuber before
 * upserting, so a VTuber can never have two live docs. Idempotent: calling
 * it repeatedly for the same externalId is a no-op beyond field refresh.
 *
 * Returns true only on a not-live -> live *edge* (i.e. this VTuber was not
 * already live on this stream), so callers can decide whether to notify.
 */
export async function markLive(
  vtuberId: mongoose.Types.ObjectId,
  stream: IStreamInput
): Promise<boolean> {
  const alreadyLive = await Stream.exists({
    platform: stream.platform,
    externalId: stream.externalId,
    status: 'live',
  });

  // Step 1 before step 2: a crash between them leaves zero live docs rather
  // than two. Zero is self-healing on the next event or poll; two is the bug.
  await Stream.updateMany(
    { vtuberId, status: 'live', externalId: { $ne: stream.externalId } },
    { status: 'ended', endTime: new Date() }
  );

  await Stream.findOneAndUpdate(
    { platform: stream.platform, externalId: stream.externalId },
    stream,
    { upsert: true, returnDocument: 'after' }
  );

  return !alreadyLive;
}

/**
 * Mark a VTuber as no longer live.
 *
 * `externalId` optional: Twitch's stream.offline doesn't name the stream, so
 * omitting it means "whatever is currently live for this VTuber". Stamps
 * endTime at call time since the sources that trigger this give no timestamp.
 *
 * Returns true only if something was actually transitioned.
 */
export async function markEnded(
  vtuberId: mongoose.Types.ObjectId,
  externalId?: string
): Promise<boolean> {
  const query: any = { vtuberId, status: 'live' };
  if (externalId) query.externalId = externalId;

  const result = await Stream.updateMany(query, { status: 'ended', endTime: new Date() });
  return result.modifiedCount > 0;
}
