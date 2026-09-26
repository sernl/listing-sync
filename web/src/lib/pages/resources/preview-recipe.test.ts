import { describe, expect, it } from 'vitest';
import { parseRecipe } from './preview-recipe';

describe('a remembered preview recipe', () => {
	it('reads back what the maker wrote', () => {
		const recipe = { pages: [3, 1], marked: [3], watermark: true, watermarkText: 'Ms Rees' };
		expect(parseRecipe(JSON.stringify(recipe))).toEqual(recipe);
	});

	// Storage is shared with other builds and scripts: anything the maker did
	// not write opens the maker on its defaults rather than on nonsense.
	it('is ignored where it is not one the maker could have written', () => {
		expect(parseRecipe(null)).toBeNull();
		expect(parseRecipe('not json')).toBeNull();
		expect(parseRecipe(JSON.stringify({ pages: [], marked: [], watermark: false, watermarkText: '' }))).toBeNull();
		expect(parseRecipe(JSON.stringify({ pages: [0], marked: [], watermark: false, watermarkText: '' }))).toBeNull();
		expect(parseRecipe(JSON.stringify({ pages: [1], marked: [], watermark: 'yes', watermarkText: '' }))).toBeNull();
	});
});
