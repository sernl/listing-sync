// The shape of the board's chip strip, read from the source rather than from a
// render.
//
// Vitest runs in `node`, so there is no component to mount and no layout to
// measure. These are the facts about the file that the type checker will not
// state and the render proofs cannot: that the marketplace is drawn by its own
// mark rather than typed out, and that the mark says nothing to a screen
// reader, because the chip's accessible name already spells the platform out
// and a mark with alternative text would make it say it twice.

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const source = readFileSync(new URL('./MarketplaceChips.svelte', import.meta.url), 'utf8');

describe('the chip strip', () => {
	it('draws each marketplace by its own mark', () => {
		expect(source).toContain('MARK_SRC[MARKETPLACE_OF[chip.inventory]]');
		// One arm draws a link and one draws a plain chip, and both carry it.
		expect(source.split('MARK_SRC[MARKETPLACE_OF[chip.inventory]]').length - 1).toBe(2);
	});

	it('no longer types the marketplace out beside it', () => {
		expect(source).not.toContain('SHORT_NAME');
	});

	it('gives every mark an empty alt, so the platform is announced once', () => {
		const images = [...source.matchAll(/<img\b[\s\S]*?\/>/g)].map((match) => match[0]);
		expect(images.length).toBeGreaterThan(0);
		for (const image of images) {
			expect(image, image).toMatch(/alt=""/);
		}
	});

	it('still spells the platform, the state and the cause out in words', () => {
		expect(source).toContain('platformTitle(chip.inventory)');
		expect(source).toContain('aria-label={described(chip)}');
	});
});
