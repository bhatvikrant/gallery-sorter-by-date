import * as path from 'path';
import * as fs from 'fs';
import { spawn } from 'child_process';
import chalk from 'chalk';
import ora from 'ora';
import { scanDirectory } from './scanner';
import { extractAllMetadata } from './metadata';
import { groupByDate } from './grouper';
import { copyFiles } from './copier';
import { OUTPUT_FOLDER_NAME } from './constants';

const LEGACY_FLAG = '--legacy';

/**
 * Resolves the absolute path of the compiled Rust binary, accounting for
 * Windows `.exe` and the fact that this file may run from `dist/` or directly
 * via ts-node from `src/`.
 */
function resolveNativeBinary(): string {
  const exe = process.platform === 'win32' ? 'gallery-sorter.exe' : 'gallery-sorter';
  // __dirname is dist/ (built) or src/ (ts-node). Walk up one level to repo root.
  const repoRoot = path.resolve(__dirname, '..');
  return path.join(repoRoot, 'native', 'target', 'release', exe);
}

/**
 * Default path: hand off to the Rust engine. We inherit stdio so progress
 * bars render directly in the user's terminal.
 */
function delegateToNative(args: string[]): never {
  const bin = resolveNativeBinary();
  if (!fs.existsSync(bin)) {
    console.error(chalk.red('✗ Native Rust engine not found at:'));
    console.error(chalk.gray(`  ${bin}`));
    console.error('');
    console.error(chalk.bold('Build it once with:'));
    console.error(chalk.cyan('  npm run build:native'));
    console.error('');
    console.error(chalk.gray(`Or run with ${chalk.bold(LEGACY_FLAG)} to use the slower Node implementation.`));
    process.exit(1);
  }
  const child = spawn(bin, args, { stdio: 'inherit' });
  child.on('error', (err) => {
    console.error(chalk.red('✗ Failed to launch native engine:'), err);
    process.exit(1);
  });
  child.on('exit', (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal);
    }
    process.exit(code ?? 0);
  });
  // spawn is async; this function never returns control to the caller.
  // Block forever to satisfy `never` return type.
  return new Promise<never>(() => undefined) as unknown as never;
}

/**
 * Formats elapsed time in a human-readable format
 */
function formatElapsedTime(startTime: number): string {
  const elapsed = (performance.now() - startTime) / 1000;
  
  if (elapsed < 1) {
    return `${(elapsed * 1000).toFixed(0)}ms`;
  }
  
  if (elapsed < 60) {
    return `${elapsed.toFixed(1)}s`;
  }
  
  const minutes = Math.floor(elapsed / 60);
  const seconds = (elapsed % 60).toFixed(0);
  return `${minutes}m ${seconds}s`;
}

/**
 * Pluralizes a word based on count
 */
function pluralize(count: number, singular: string, plural?: string): string {
  return count === 1 ? singular : (plural || `${singular}s`);
}

/**
 * Main entry point.
 *
 * Routes:
 *   - `--legacy` present  → run this TypeScript pipeline (kept intact for
 *                            compatibility / fallback / debugging).
 *   - default             → delegate to the compiled Rust engine in
 *                            `native/target/release/gallery-sorter` (much
 *                            faster on large libraries; see README).
 */
async function main(): Promise<void> {
  const cliArgs = process.argv.slice(2);
  const legacyIdx = cliArgs.indexOf(LEGACY_FLAG);

  if (legacyIdx === -1) {
    delegateToNative(cliArgs);
    return;
  }
  cliArgs.splice(legacyIdx, 1);

  const totalStartTime = performance.now();

  const sourceDir = cliArgs[0] || process.cwd();
  const absoluteSourceDir = path.resolve(sourceDir);

  console.log(chalk.bold.blue('\n📁 Media Gallery Sorter ') + chalk.gray('(legacy Node engine)\n'));
  console.log(chalk.gray(`Source: ${absoluteSourceDir}\n`));
  
  // Step 1: Scan directory
  const scanSpinner = ora({ text: 'Scanning for media files...', stream: process.stdout }).start();
  const scanStartTime = performance.now();
  
  let scanResult;
  try {
    scanResult = await scanDirectory(absoluteSourceDir);
  } catch (error) {
    scanSpinner.fail(chalk.red('Failed to scan directory'));
    console.error(error);
    process.exit(1);
  }
  
  if (scanResult.files.length === 0) {
    scanSpinner.warn(chalk.yellow('No media files found'));
    process.exit(0);
  }
  
  const dirWord = pluralize(scanResult.directoryCount, 'directory', 'directories');
  scanSpinner.succeed(
    chalk.green(`Found ${chalk.bold(scanResult.imageCount)} ${pluralize(scanResult.imageCount, 'image')} and ${chalk.bold(scanResult.videoCount)} ${pluralize(scanResult.videoCount, 'video')} `) +
    chalk.gray(`across ${scanResult.directoryCount} ${dirWord} `) +
    chalk.gray(`(${formatElapsedTime(scanStartTime)})`)
  );
  
  // Step 2: Extract metadata
  const metadataSpinner = ora({ text: 'Extracting metadata...', stream: process.stdout }).start();
  const metadataStartTime = performance.now();
  
  let metadata;
  try {
    metadata = await extractAllMetadata(scanResult.files, (current, total) => {
      metadataSpinner.text = `Extracting metadata... ${current}/${total}`;
    });
  } catch (error) {
    metadataSpinner.fail(chalk.red('Failed to extract metadata'));
    console.error(error);
    process.exit(1);
  }
  
  metadataSpinner.succeed(
    chalk.green(`Processed ${chalk.bold(metadata.length)} ${pluralize(metadata.length, 'file')} `) +
    chalk.gray(`(${formatElapsedTime(metadataStartTime)})`)
  );
  
  // Step 3: Group by date
  const groupResult = groupByDate(metadata);
  
  // Display groups
  console.log(chalk.bold('\nGrouping by date...'));
  for (const group of groupResult.groups) {
    const parts: string[] = [];
    if (group.imageCount > 0) {
      parts.push(`${group.imageCount} ${pluralize(group.imageCount, 'image')}`);
    }
    if (group.videoCount > 0) {
      parts.push(`${group.videoCount} ${pluralize(group.videoCount, 'video')}`);
    }
    console.log(chalk.gray(`├── ${chalk.white(group.displayName)} (${parts.join(', ')})`));
  }
  
  // Step 4: Copy files to output folder
  const outputDir = path.join(absoluteSourceDir, OUTPUT_FOLDER_NAME);
  console.log(chalk.bold('\nCopying files...'));
  console.log(chalk.gray(`Output: ${outputDir}\n`));
  const copySpinner = ora({ text: 'Starting copy...', stream: process.stdout }).start();
  const copyStartTime = performance.now();
  
  let copyResult;
  try {
    copyResult = await copyFiles(groupResult.groups, outputDir, (current, total, filename) => {
      copySpinner.text = `Copying ${current}/${total}: ${filename}`;
    });
  } catch (error) {
    copySpinner.fail(chalk.red('Failed to copy files'));
    console.error(error);
    process.exit(1);
  }
  
  const copyParts: string[] = [
    chalk.green(`Created ${chalk.bold(copyResult.foldersCreated)} ${pluralize(copyResult.foldersCreated, 'folder')}`),
    chalk.green(`copied ${chalk.bold(copyResult.filesCopied)} ${pluralize(copyResult.filesCopied, 'file')}`)
  ];
  
  if (copyResult.filesSkipped > 0) {
    copyParts.push(chalk.gray(`skipped ${copyResult.filesSkipped} ${pluralize(copyResult.filesSkipped, 'duplicate')}`));
  }
  
  copySpinner.succeed(
    copyParts.join(', ') +
    chalk.gray(` (${formatElapsedTime(copyStartTime)})`)
  );
  
  // Summary
  console.log(chalk.bold.green('\n✓ Done! ') + chalk.gray(`Media organized in ${outputDir}`));
  console.log(chalk.gray(`Total time: ${formatElapsedTime(totalStartTime)}\n`));
}

// Run the main function
main().catch((error) => {
  console.error(chalk.red('An unexpected error occurred:'), error);
  process.exit(1);
});
