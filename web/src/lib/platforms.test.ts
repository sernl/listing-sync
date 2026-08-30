import { describe, expect, it } from 'vitest';
import { AUTHORABLE_PLATFORMS, platformTitle } from './platforms';

describe('how a platform is named', () => {
	it('leads with the acronym and its full name, as the founder settled it', () => {
		expect(platformTitle('Tpt')).toBe('TPT (Teachers Pay Teachers)');
	});

	it('separates the three Tes sites, which are one marketplace under three inventories', () => {
		const titles = (['TesGb', 'TesUs', 'TesNz'] as const).map(platformTitle);
		expect(new Set(titles).size).toBe(3);
		for (const title of titles) {
			expect(title.startsWith('TES (Tes.com)')).toBe(true);
		}
	});

	it('offers only the platforms an adapter exists for', () => {
		expect(AUTHORABLE_PLATFORMS).toEqual(['Tpt', 'TesGb', 'TesUs', 'TesNz']);
	});
});
