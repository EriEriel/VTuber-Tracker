import './polyfill';
import mongoose from 'mongoose';

console.log('Mongoose imported successfully! Ready state:', mongoose.connection.readyState);
process.exit(0);
