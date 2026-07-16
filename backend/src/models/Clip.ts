import mongoose, { Schema, Document } from 'mongoose';
import { z } from 'zod';

export const ClipZodSchema = z.object({
  vtuberId: z.string(),
  sourceStreamId: z.string().optional().nullable(),
  externalId: z.string().min(1),
  title: z.string().min(1),
  url: z.string().url(),
  viewCount: z.number().default(0),
  createdAt: z.union([z.date(), z.string().transform((v) => new Date(v))]),
  sourceApi: z.enum(['holodex', 'youtube_api', 'twitch_api']),
});

export type IClipInput = z.infer<typeof ClipZodSchema>;

export interface IClip extends Omit<IClipInput, 'vtuberId' | 'sourceStreamId' | 'createdAt'>, Document {
  vtuberId: mongoose.Types.ObjectId;
  sourceStreamId: mongoose.Types.ObjectId | null;
  createdAt: Date;
  updatedAt: Date;
}

const ClipSchema = new Schema<IClip>(
  {
    vtuberId: { type: Schema.Types.ObjectId, ref: 'VTuber', required: true },
    sourceStreamId: { type: Schema.Types.ObjectId, ref: 'Stream', default: null },
    externalId: { type: String, required: true },
    title: { type: String, required: true },
    url: { type: String, required: true },
    viewCount: { type: Number, default: 0 },
    createdAt: { type: Date, required: true },
    sourceApi: { type: String, enum: ['holodex', 'youtube_api', 'twitch_api'], required: true },
  },
  {
    timestamps: true,
  }
);

// Compound index to ensure uniqueness of clips per source API + external ID
ClipSchema.index({ sourceApi: 1, externalId: 1 }, { unique: true });
ClipSchema.index({ vtuberId: 1 });

export const Clip = mongoose.models.Clip || mongoose.model<IClip>('Clip', ClipSchema);
