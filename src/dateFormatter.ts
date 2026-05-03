import * as path from 'path';
import { format, parse, isValid } from 'date-fns';

/**
 * Gets the ordinal suffix for a day number (st, nd, rd, th)
 */
function getOrdinalSuffix(day: number): string {
  if (day >= 11 && day <= 13) {
    return 'th';
  }
  
  switch (day % 10) {
    case 1: return 'st';
    case 2: return 'nd';
    case 3: return 'rd';
    default: return 'th';
  }
}

/**
 * Formats a day with ordinal suffix (1st, 2nd, 3rd, etc.)
 */
function formatDayWithOrdinal(date: Date): string {
  const day = date.getDate();
  return `${day}${getOrdinalSuffix(day)}`;
}

/**
 * Formats a date as a hierarchical folder path: "2025/1. Jan/15th"
 */
export function formatDateForFolder(date: Date): string {
  const year = format(date, 'yyyy');
  const monthIndex = format(date, 'M'); // 1-12
  const monthName = format(date, 'MMM'); // Jan, Feb, etc.
  const dayWithOrdinal = formatDayWithOrdinal(date);
  
  return path.join(year, `${monthIndex}. ${monthName}`, dayWithOrdinal);
}

/**
 * Formats a date for display: "15th Jan 2025"
 */
export function formatDateForDisplay(date: Date): string {
  const dayWithOrdinal = formatDayWithOrdinal(date);
  const monthYear = format(date, 'MMM yyyy');
  
  return `${dayWithOrdinal} ${monthYear}`;
}

/**
 * Gets a date key string for grouping (YYYY-MM-DD format)
 */
export function getDateKey(date: Date): string {
  return format(date, 'yyyy-MM-dd');
}

/**
 * Parses a date key string back to a Date object
 */
export function parseDateKey(dateKey: string): Date {
  const parsed = parse(dateKey, 'yyyy-MM-dd', new Date());
  if (!isValid(parsed)) {
    throw new Error(`Invalid date key: ${dateKey}`);
  }
  return parsed;
}

/**
 * Parses EXIF date string format (YYYY:MM:DD HH:MM:SS) to Date object
 */
export function parseExifDate(dateString: string): Date | null {
  if (!dateString) return null;
  
  // EXIF date format: "2025:01:10 14:30:00"
  const parsed = parse(dateString, 'yyyy:MM:dd HH:mm:ss', new Date());
  
  return isValid(parsed) ? parsed : null;
}
