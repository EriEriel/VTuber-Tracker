import mongoose, { Schema, Document } from 'mongoose';

// Singleton document holding the user access token EventSub's websocket
// transport requires (unlike webhook transport, which uses the app token
// in twitch-token.ts). Always queried/updated with an empty filter since
// there's only ever one Twitch account authorized per install.
export interface ITwitchAuth extends Document {
  accessToken: string;
  refreshToken: string;
  expiresAt: Date;
}

const TwitchAuthSchema = new Schema<ITwitchAuth>(
  {
    accessToken: { type: String, required: true },
    refreshToken: { type: String, required: true },
    expiresAt: { type: Date, required: true },
  },
  {
    timestamps: true,
  }
);

export const TwitchAuth = mongoose.models.TwitchAuth || mongoose.model<ITwitchAuth>('TwitchAuth', TwitchAuthSchema);
