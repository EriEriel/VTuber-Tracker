import mongoose from 'mongoose';
import { connectToDatabase } from './lib/db';

async function test() {
  console.log('Connecting...');
  try {
    await connectToDatabase();
    console.log('Connect resolved. Ready state:', mongoose.connection.readyState);
    await new Promise(r => setTimeout(r, 1000));
    console.log('After 1s. Ready state:', mongoose.connection.readyState);
  } catch (err) {
    console.error('Connection error in test:', err);
  }
  process.exit(0);
}

test();
