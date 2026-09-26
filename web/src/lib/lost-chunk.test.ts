import { describe, expect, it } from 'vitest';
import { isLostChunk, mayReloadForLostChunk } from './lost-chunk';

function store(): Pick<Storage, 'getItem' | 'setItem'> {
	const held = new Map<string, string>();
	return {
		getItem: (key) => held.get(key) ?? null,
		setItem: (key, value) => void held.set(key, value)
	};
}

describe('a lost chunk', () => {
	it('is recognised in the words each engine uses', () => {
		expect(
			isLostChunk(
				new TypeError(
					'Failed to fetch dynamically imported module: https://teachouse.io/_app/immutable/nodes/50.js'
				)
			)
		).toBe(true);
		expect(isLostChunk(new TypeError('Importing a module script failed.'))).toBe(true);
		expect(isLostChunk(new Error('Unable to preload CSS for /_app/immutable/assets/x.css'))).toBe(
			true
		);
	});

	it('is not any other failure, so a real bug still shows its cause', () => {
		expect(isLostChunk(new TypeError('Object.hasOwn is not a function'))).toBe(false);
		expect(isLostChunk(new TypeError('Failed to fetch'))).toBe(false);
		expect(isLostChunk(null)).toBe(false);
	});

	it('is reloaded for once per window and then shown', () => {
		const held = store();
		expect(mayReloadForLostChunk(held, 1_000)).toBe(true);
		expect(mayReloadForLostChunk(held, 20_000)).toBe(false);
		expect(mayReloadForLostChunk(held, 31_001)).toBe(true);
	});

	it('treats a corrupt record as no record', () => {
		const held = store();
		held.setItem('teachouse.reloaded-for-lost-chunk', 'yesterday');
		expect(mayReloadForLostChunk(held, 5_000)).toBe(true);
	});
});
