import mongoose, { Schema, Document } from 'mongoose';
import { z } from 'zod';

export const VTuberPlatformSchema = z.enum(['youtube', 'twitch']);
export const VTuberSourceSchema = z.enum(['holodex', 'youtube_api', 'twitch_api']);

export const VTuberZodSchema = z.object({
  name: z.string().min(1),
  englishName: z.string().optional(),
  photo: z.string().url().optional().or(z.string().length(0)),
  platform: VTuberPlatformSchema,
  source: VTuberSourceSchema,
  platformChannelId: z.string().min(1),
  org: z.string().optional(),
  suborg: z.string().optional(),
  isTracked: z.boolean().default(true),
  lastSyncedAt: z.date().nullable().default(null),
  lastLiveSyncedAt: z.date().nullable().default(null),
  lastStatsSyncedAt: z.date().nullable().default(null),
});

export type IVTuberInput = z.infer<typeof VTuberZodSchema>;

export interface IVTuber extends Omit<IVTuberInput, 'lastSyncedAt' | 'lastLiveSyncedAt' | 'lastStatsSyncedAt'>, Document {
  lastSyncedAt: Date | null;
  lastLiveSyncedAt: Date | null;
  lastStatsSyncedAt: Date | null;
  createdAt: Date;
  updatedAt: Date;
}

const VTuberSchema = new Schema<IVTuber>(
  {
    name: { type: String, required: true },
    englishName: { type: String },
    photo: { type: String },
    platform: { type: String, enum: ['youtube', 'twitch'], required: true },
    source: { type: String, enum: ['holodex', 'youtube_api', 'twitch_api'], required: true },
    platformChannelId: { type: String, required: true },
    org: { type: String },
    suborg: { type: String },
    isTracked: { type: Boolean, default: true, required: true },
    lastSyncedAt: { type: Date, default: null },
    lastLiveSyncedAt: { type: Date, default: null },
    lastStatsSyncedAt: { type: Date, default: null },
  },
  {
    timestamps: true,
  }
);

// Compound index to ensure uniqueness per platform
VTuberSchema.index({ platform: 1, platformChannelId: 1 }, { unique: true });

export const VTuber = mongoose.models.VTuber || mongoose.model<IVTuber>('VTuber', VTuberSchema);
