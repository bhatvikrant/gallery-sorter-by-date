import * as fs from 'fs/promises';
import { constants as fsConstants } from 'fs';
import ExifReader from 'exifreader';
import ffmpeg from 'fluent-ffmpeg';
import pLimit from 'p-limit';
import { parseExifDate } from './dateFormatter';
import { CONCURRENCY_LIMIT } from './constants';
import { MediaFile, MediaMetadata, DateSource, ProgressCallback } from './types';

/**
 * Checks if a file exists
 */
async function fileExists(filePath: string): Promise<boolean> {
  try {
    await fs.access(filePath, fsConstants.F_OK);
    return true;
  } catch {
    return false;
  }
}

/**
 * Extracts date from image EXIF metadata
 */
async function extractImageDate(filePath: string): Promise<{ date: Date; source: DateSource }> {
  try {
    const buffer = await fs.readFile(filePath);
    const tags = ExifReader.load(buffer, { expanded: true });
    
    // Access exif tags with type safety
    const exifTags = tags.exif as Record<string, { description?: string } | undefined> | undefined;
    
    // Try DateTimeOriginal first (when photo was actually taken)
    const dateTimeOriginal = exifTags?.DateTimeOriginal?.description;
    if (dateTimeOriginal) {
      const date = parseExifDate(dateTimeOriginal);
      if (date) return { date, source: 'exif' };
    }
    
    // Try CreateDate as fallback (also known as DateTimeDigitized in some cameras)
    const createDate = exifTags?.CreateDate?.description || exifTags?.DateTimeDigitized?.description;
    if (createDate) {
      const date = parseExifDate(createDate);
      if (date) return { date, source: 'exif' };
    }
    
    // Try DateTime as another fallback
    const dateTime = exifTags?.DateTime?.description;
    if (dateTime) {
      const date = parseExifDate(dateTime);
      if (date) return { date, source: 'exif' };
    }
  } catch {
    // EXIF extraction failed, fall back to file system dates
  }
  
  // Fallback to file system dates
  return extractFilesystemDate(filePath);
}

/**
 * Extracts date from video metadata using ffprobe
 */
async function extractVideoDate(filePath: string): Promise<{ date: Date; source: DateSource }> {
  return new Promise((resolve) => {
    ffmpeg.ffprobe(filePath, async (err, metadata) => {
      if (!err && metadata) {
        // Try to get creation_time from format tags
        const creationTime = metadata.format?.tags?.creation_time;
        if (creationTime) {
          const date = new Date(creationTime);
          if (!isNaN(date.getTime())) {
            resolve({ date, source: 'video_metadata' });
            return;
          }
        }
        
        // Try QuickTime specific tag (for MOV files from iPhones)
        const quicktimeDate = metadata.format?.tags?.['com.apple.quicktime.creationdate'];
        if (quicktimeDate) {
          const date = new Date(quicktimeDate);
          if (!isNaN(date.getTime())) {
            resolve({ date, source: 'video_metadata' });
            return;
          }
        }
      }
      
      // Fallback to file system dates
      const result = await extractFilesystemDate(filePath);
      resolve(result);
    });
  });
}

/**
 * Extracts date from file system metadata
 */
async function extractFilesystemDate(filePath: string): Promise<{ date: Date; source: DateSource }> {
  const stats = await fs.stat(filePath);
  
  // Prefer birthtime (file creation date) if available
  if (stats.birthtime && stats.birthtime.getTime() > 0) {
    return { date: stats.birthtime, source: 'file_birthtime' };
  }
  
  // Fall back to modification time
  return { date: stats.mtime, source: 'file_mtime' };
}

/**
 * Extracts metadata from a single media file
 */
export async function extractMetadata(file: MediaFile): Promise<MediaMetadata> {
  const result = file.type === 'image' 
    ? await extractImageDate(file.path)
    : await extractVideoDate(file.path);
  
  return {
    file,
    createdDate: result.date,
    dateSource: result.source
  };
}

/**
 * Extracts metadata from multiple media files with parallel processing
 */
export async function extractAllMetadata(
  files: MediaFile[],
  onProgress?: ProgressCallback
): Promise<MediaMetadata[]> {
  const limit = pLimit(CONCURRENCY_LIMIT);
  let completed = 0;
  
  const promises = files.map(file => 
    limit(async () => {
      const metadata = await extractMetadata(file);
      completed++;
      onProgress?.(completed, files.length);
      return metadata;
    })
  );
  
  return Promise.all(promises);
}
