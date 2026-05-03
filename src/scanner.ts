import { glob } from 'glob';
import * as path from 'path';
import { IMAGE_EXTENSIONS, VIDEO_EXTENSIONS, ALL_MEDIA_EXTENSIONS } from './constants';
import { MediaFile, MediaType, ScanResult } from './types';

/**
 * Determines if a file is an image based on its extension
 */
export function isImage(filePath: string): boolean {
  const ext = path.extname(filePath).toLowerCase();
  return (IMAGE_EXTENSIONS as readonly string[]).includes(ext);
}

/**
 * Determines if a file is a video based on its extension
 */
export function isVideo(filePath: string): boolean {
  const ext = path.extname(filePath).toLowerCase();
  return (VIDEO_EXTENSIONS as readonly string[]).includes(ext);
}

/**
 * Gets the media type of a file
 */
export function getMediaType(filePath: string): MediaType | null {
  if (isImage(filePath)) return 'image';
  if (isVideo(filePath)) return 'video';
  return null;
}

/**
 * Creates a glob pattern for all supported media formats
 */
function createGlobPattern(): string {
  const patterns = ALL_MEDIA_EXTENSIONS.map(ext => ext.slice(1)); // Remove leading dot
  // Create pattern for both lowercase and uppercase extensions
  return `**/*.{${patterns.join(',')},${patterns.map(p => p.toUpperCase()).join(',')}}`;
}

/**
 * Scans a directory recursively for all media files
 */
export async function scanDirectory(dirPath: string): Promise<ScanResult> {
  const pattern = createGlobPattern();
  const absolutePath = path.resolve(dirPath);
  
  const files = await glob(pattern, {
    cwd: absolutePath,
    nodir: true,
    absolute: true
  });

  const mediaFiles: MediaFile[] = [];
  const directories = new Set<string>();
  let imageCount = 0;
  let videoCount = 0;

  for (const filePath of files) {
    const mediaType = getMediaType(filePath);
    if (!mediaType) continue;

    const mediaFile: MediaFile = {
      path: filePath,
      filename: path.basename(filePath),
      extension: path.extname(filePath).toLowerCase(),
      type: mediaType
    };

    mediaFiles.push(mediaFile);
    directories.add(path.dirname(filePath));

    if (mediaType === 'image') {
      imageCount++;
    } else {
      videoCount++;
    }
  }

  return {
    files: mediaFiles,
    imageCount,
    videoCount,
    directoryCount: directories.size
  };
}
