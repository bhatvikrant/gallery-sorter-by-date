/**
 * Media type classification
 */
export type MediaType = 'image' | 'video';

/**
 * Source of the extracted date
 */
export type DateSource = 'exif' | 'video_metadata' | 'file_birthtime' | 'file_mtime';

/**
 * Represents a media file found during scanning
 */
export interface MediaFile {
  path: string;
  filename: string;
  extension: string;
  type: MediaType;
}

/**
 * Result of scanning a directory for media files
 */
export interface ScanResult {
  files: MediaFile[];
  imageCount: number;
  videoCount: number;
  directoryCount: number;
}

/**
 * Metadata extracted from a media file
 */
export interface MediaMetadata {
  file: MediaFile;
  createdDate: Date;
  dateSource: DateSource;
}

/**
 * Group of media files sharing the same date
 */
export interface DateGroup {
  dateKey: string;
  folderPath: string;
  displayName: string;
  date: Date;
  files: MediaMetadata[];
  imageCount: number;
  videoCount: number;
}

/**
 * Result of grouping media files by date
 */
export interface GroupResult {
  groups: DateGroup[];
  totalImages: number;
  totalVideos: number;
}

/**
 * Result of copying files to destination
 */
export interface CopyResult {
  foldersCreated: number;
  filesCopied: number;
  filesSkipped: number;
}

/**
 * Progress callback function type
 */
export type ProgressCallback = (current: number, total: number, filename?: string) => void;

