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
        console.error('Failed to connect to MongoDB:', error);
        throw error;
      });
  }
  return connectionPromise;
}
