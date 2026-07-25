import mongoose from 'mongoose';

const MONGODB_URI = process.env.MONGODB_URI!;
let connectionPromise: Promise<typeof mongoose> | null = null;

export async function connectToDatabase(): Promise<typeof mongoose> {
  if (mongoose.connection.readyState === 1) {
    return mongoose;
  }
  if (!connectionPromise) {
    connectionPromise = mongoose.connect(MONGODB_URI)
      .then((conn) => {
        console.log('MongoDB connected successfully');
        return conn;
      })
      .catch((error) => {
        connectionPromise = null; // allow retry on next call
        // Only the message, not the error object: Mongoose attaches a full
        // TopologyDescription for every replica-set member, so logging the
        // object turns one failure into a screenful — and under a
        // restarting container, one screenful per restart. The message
        // itself is the useful part (it names the IP-whitelist case
        // explicitly, which is the usual cause when the host changes).
        console.error(
          'Failed to connect to MongoDB:',
          error instanceof Error ? error.message : error
        );
        // Still thrown so callers decide what to do — index.ts exits,
        // while the tests need the rejection to fail the run.
        throw error;
      });
  }
  return connectionPromise;
}
