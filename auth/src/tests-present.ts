/**
 * Fail before `node --test` runs if the list of test files it is given has
 * drifted from the test files on disk.
 *
 * `node --test` drops a named file that does not exist without a word on either
 * stream, and exits 0 as long as one of the others ran. So a renamed or moved
 * test file takes its assertions out of the lane silently, which is the same
 * hole a glob had for a different reason. Naming the files is what makes them
 * checkable; this is the check.
 */
import { readdirSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

const named = (): readonly string[] => {
  const parsed: unknown = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));
  const scripts =
    typeof parsed === 'object' && parsed !== null
      ? (parsed as { scripts?: unknown }).scripts
      : undefined;
  const script =
    typeof scripts === 'object' && scripts !== null
      ? (scripts as { test?: unknown }).test
      : undefined;
  return typeof script === 'string' ? (script.match(/src\/[\w./-]+\.test\.ts/gu) ?? []) : [];
};

const onDisk = (directory: string, prefix: string): readonly string[] =>
  readdirSync(directory, { withFileTypes: true }).flatMap((entry) =>
    entry.isDirectory()
      ? onDisk(join(directory, entry.name), `${prefix}${entry.name}/`)
      : entry.name.endsWith('.test.ts')
        ? [`${prefix}${entry.name}`]
        : [],
  );

const listed = [...named()].sort();
const present = [...onDisk(join(root, 'src'), 'src/')].sort();

if (listed.length === 0) {
  console.error('tam-auth: the test script names no test files');
  process.exit(1);
}

if (listed.join('\n') !== present.join('\n')) {
  console.error('tam-auth: the test script and the test files on disk disagree.');
  console.error(`  named in package.json: ${listed.join(', ')}`);
  console.error(`  present in src:        ${present.join(', ')}`);
  console.error('Add the new file to the "test" script, or restore the one that moved.');
  process.exit(1);
}
