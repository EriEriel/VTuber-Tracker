import mongoose, { Schema, Document } from 'mongoose';
import { z } from 'zod';

export const StreamStatusSchema = z.enum(['upcoming', 'live', 'ended', 'unknown']);

export const StreamZodSchema = z.object({
  vtuberId: z.string(),
  externalId: z.string().min(1),
  title: z.string().min(1),
  platform: z.enum(['youtube', 'twitch']),
  startTime: z.union([z.date(), z.string().transform((v) => new Date(v))]),
  endTime: z.union([z.date(), z.string().transform((v) => new Date(v))]).optional().nullable(),
  duration: z.number().optional().nullable(),
  status: StreamStatusSchema,
  url: z.string().url(),
  thumbnailUrl: z.string().url().optional().nullable().or(z.string().length(0)),
  sourceApi: z.enum(['holodex', 'youtube_api', 'twitch_api']),
});

export type IStreamInput = z.infer<typeof StreamZodSchema>;

export interface IStream extends Omit<IStreamInput, 'vtuberId' | 'startTime' | 'endTime'>, Document {
  vtuberId: mongoose.Types.ObjectId;
  startTime: Date;
  endTime: Date | null;
  createdAt: Date;
  updatedAt: Date;
}

const StreamSchema = new Schema<IStream>(
  {
    vtuberId: { type: Schema.Types.ObjectId, ref: 'VTuber', required: true },
    externalId: { type: String, required: true },
    title: { type: String, required: true },
    platform: { type: String, enum: ['youtube', 'twitch'], required: true },
    startTime: { type: Date, required: true },
    endTime: { type: Date, default: null },
    duration: { type: Number, default: null },
    status: { type: String, enum: ['upcoming', 'live', 'ended', 'unknown'], required: true },
    url: { type: String, required: true },
    thumbnailUrl: { type: String },
    sourceApi: { type: String, enum: ['holodex', 'youtube_api', 'twitch_api'], required: true },
  },
  {
    timestamps: true,
  }
);

// Compound index to ensure uniqueness per platform and external ID
StreamSchema.index({ platform: 1, externalId: 1 }, { unique: true });
// Index on vtuberId and status for filtering/aggregation
StreamSchema.index({ vtuberId: 1, status: 1 });

export const Stream = mongoose.models.Stream || mongoose.model<IStream>('Stream', StreamSchema);
