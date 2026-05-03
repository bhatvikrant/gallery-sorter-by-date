import * as fs from 'fs/promises';
import { constants as fsConstants } from 'fs';
import * as path from 'path';
import { DateGroup, CopyResult, ProgressCallback } from './types';

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
 * Generates a unique filename if the file already exists
 */
async function getUniqueFilename(destDir: string, filename: string): Promise<string> {
  const ext = path.extname(filename);
  const basename = path.basename(filename, ext);
  let newFilename = filename;
  let counter = 1;
  
  while (await fileExists(path.join(destDir, newFilename))) {
    newFilename = `${basename}_${counter}${ext}`;
    counter++;
  }
  
  return newFilename;
}

/**
 * Checks if two files are the same based on size
 */
async function filesAreEqual(src: string, dest: string): Promise<boolean> {
  if (!(await fileExists(dest))) return false;
  
  const [srcStats, destStats] = await Promise.all([
    fs.stat(src),
    fs.stat(dest)
  ]);
  
  return srcStats.size === destStats.size;
}

/**
 * Ensures a directory exists, creating it if necessary
 */
async function ensureDir(dirPath: string): Promise<boolean> {
  try {
    await fs.access(dirPath, fsConstants.F_OK);
    return false; // Directory already existed
  } catch {
    await fs.mkdir(dirPath, { recursive: true });
    return true; // Directory was created
  }
}

/**
 * Copies files from groups to their destination folders
 */
export async function copyFiles(
  groups: DateGroup[],
  destBase: string,
  onProgress?: ProgressCallback
): Promise<CopyResult> {
  let foldersCreated = 0;
  let filesCopied = 0;
  let filesSkipped = 0;
  
  // Calculate total files for progress
  const totalFiles = groups.reduce((sum, g) => sum + g.files.length, 0);
  let currentFile = 0;
  
  for (const group of groups) {
    const destDir = path.join(destBase, group.folderPath);
    
    // Create destination folder if it doesn't exist
    const wasCreated = await ensureDir(destDir);
    if (wasCreated) {
      foldersCreated++;
    }
    
    // Copy each file in the group
    for (const metadata of group.files) {
      currentFile++;
      const srcPath = metadata.file.path;
      const filename = metadata.file.filename;
      
      // Check if file already exists at destination with same size
      const destPath = path.join(destDir, filename);
      if (await filesAreEqual(srcPath, destPath)) {
        filesSkipped++;
        onProgress?.(currentFile, totalFiles, filename);
        continue;
      }
      
      // Get unique filename if needed
      const uniqueFilename = await getUniqueFilename(destDir, filename);
      const finalDestPath = path.join(destDir, uniqueFilename);
      
      // Copy the file
      await fs.copyFile(srcPath, finalDestPath);
      filesCopied++;
      
      onProgress?.(currentFile, totalFiles, uniqueFilename);
    }
  }
  
  return {
    foldersCreated,
    filesCopied,
    filesSkipped
  };
}
