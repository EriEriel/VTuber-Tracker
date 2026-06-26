import mongoose, { Schema, Document } from 'mongoose';
import { z } from 'zod';

export const StatSnapshotZodSchema = z.object({
  vtuberId: z.string(),
  subscriberCount: z.number().nonnegative(),
  viewCount: z.number().nonnegative(),
  capturedAt: z.union([z.date(), z.string().transform((v) => new Date(v))]).default(() => new Date()),
  sourceApi: z.enum(['holodex', 'youtube_api', 'twitch_api']),
});

export type IStatSnapshotInput = z.infer<typeof StatSnapshotZodSchema>;

export interface IStatSnapshot extends Omit<IStatSnapshotInput, 'vtuberId' | 'capturedAt'>, Document {
  vtuberId: mongoose.Types.ObjectId;
  capturedAt: Date;
}

const StatSnapshotSchema = new Schema<IStatSnapshot>(
  {
    vtuberId: { type: Schema.Types.ObjectId, ref: 'VTuber', required: true },
    subscriberCount: { type: Number, required: true },
    viewCount: { type: Number, required: true },
    capturedAt: { type: Date, default: Date.now, required: true },
    sourceApi: { type: String, enum: ['holodex', 'youtube_api', 'twitch_api'], required: true },
  },
  {
    timestamps: false,
  }
);

// Index on vtuberId and capturedAt for fast time-series retrieval
StatSnapshotSchema.index({ vtuberId: 1, capturedAt: -1 });

export const StatSnapshot = mongoose.models.StatSnapshot || mongoose.model<IStatSnapshot>('StatSnapshot', StatSnapshotSchema);
