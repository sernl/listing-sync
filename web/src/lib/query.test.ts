import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

// Every file under src/ that could declare a query, read once.
function sources(dir: string): string[] {
	const out: string[] = [];
	for (const name of readdirSync(dir)) {
		const path = join(dir, name);
		if (statSync(path).isDirectory()) {
			out.push(...sources(path));
		} else if (/\.(svelte|ts)$/.test(name) && !name.endsWith('.test.ts')) {
			out.push(path);
		}
	}
	return out;
}

/** A shared query key names one shape.
 *
 * TanStack caches by key, so two readers of one key that store different
 * shapes hand each other the wrong one: the Status page stored the devices
 * array under `queryKeys.devices` while Settings stored the whole view, and
 * whichever read second found the other's shape and could not be drawn. The
 * pin is on the `queryFn` text beside each key, which is the one place the
 * shape is decided; a key with one reader is free to change. */
describe('shared query keys', () => {
	it('cache one shape each', () => {
		const shapes = new Map<string, Set<string>>();
		for (const path of sources(new URL('..', import.meta.url).pathname)) {
			const text = readFileSync(path, 'utf8');
			for (const match of text.matchAll(
				/queryKey:\s*queryKeys\.(\w+)\s*,\s*queryFn:\s*([^\n]*?)(?:,?\s*\n|\s*\}\)\))/g
			)) {
				const [, key, fn] = match;
				const set = shapes.get(key) ?? new Set();
				set.add(fn.replace(/\s+/g, ' ').replace(/,\s*$/, '').trim());
				shapes.set(key, set);
			}
		}
		const disagreeing = [...shapes.entries()]
			.filter(([, fns]) => fns.size > 1)
			.map(([key, fns]) => `${key}: ${[...fns].join(' | ')}`);
		expect(disagreeing, 'every reader of a key stores what the others store').toEqual([]);
	});
});
