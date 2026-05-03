import { getDateKey, formatDateForFolder, formatDateForDisplay, parseDateKey } from './dateFormatter';
import { MediaMetadata, DateGroup, GroupResult } from './types';

/**
 * Groups media files by their creation date
 */
export function groupByDate(files: MediaMetadata[]): GroupResult {
  const groupMap = new Map<string, MediaMetadata[]>();
  
  // Group files by date key
  for (const file of files) {
    const dateKey = getDateKey(file.createdDate);
    
    if (!groupMap.has(dateKey)) {
      groupMap.set(dateKey, []);
    }
    groupMap.get(dateKey)!.push(file);
  }
  
  // Convert map to sorted array of DateGroup objects
  const groups: DateGroup[] = [];
  let totalImages = 0;
  let totalVideos = 0;
  
  for (const [dateKey, groupFiles] of groupMap.entries()) {
    const date = parseDateKey(dateKey);
    const imageCount = groupFiles.filter(f => f.file.type === 'image').length;
    const videoCount = groupFiles.filter(f => f.file.type === 'video').length;
    
    totalImages += imageCount;
    totalVideos += videoCount;
    
    groups.push({
      dateKey,
      folderPath: formatDateForFolder(date),
      displayName: formatDateForDisplay(date),
      date,
      files: groupFiles,
      imageCount,
      videoCount
    });
  }
  
  // Sort groups by date (oldest first)
  groups.sort((a, b) => a.date.getTime() - b.date.getTime());
  
  return {
    groups,
    totalImages,
    totalVideos
  };
}
