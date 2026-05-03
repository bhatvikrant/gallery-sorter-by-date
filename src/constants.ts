/**
 * Supported image file extensions
 */
export const IMAGE_EXTENSIONS = [
  // Common formats
  '.jpg', '.jpeg', '.png', '.gif', '.webp', '.bmp', '.tiff', '.tif',
  // Apple HEIC
  '.heic', '.heif',
  // RAW formats
  '.cr2', '.cr3', '.nef', '.arw', '.orf', '.rw2', '.dng', '.raf'
] as const;

/**
 * Supported video file extensions
 */
export const VIDEO_EXTENSIONS = [
  // Common formats
  '.mp4', '.mov', '.avi', '.mkv', '.wmv', '.flv', '.webm',
  // Mobile formats
  '.3gp', '.m4v'
] as const;

/**
 * All supported media extensions
 */
export const ALL_MEDIA_EXTENSIONS = [...IMAGE_EXTENSIONS, ...VIDEO_EXTENSIONS] as const;

/**
 * Output folder name
 */
export const OUTPUT_FOLDER_NAME = 'sorted by date';

/**
 * Concurrency limit for parallel operations
 */
export const CONCURRENCY_LIMIT = 10;

