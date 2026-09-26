import { describe, expect, it } from 'vitest';
import { MAPPING, TEMPLATES, tabsOf } from './tabs';
import { OVERRIDABLE } from '$lib/templates';

describe('the two tabs', () => {
	it('names the templates first, which is where the page lands', () => {
		const tabs = tabsOf({ templates: 0, mappings: 3 });
		expect(tabs.map((tab) => tab.id)).toEqual([TEMPLATES, MAPPING]);
	});

	it('counts each tab from its own list, so neither reports the other', () => {
		expect(tabsOf({ templates: 5, mappings: 3 }).map((tab) => tab.count)).toEqual([5, 3]);
	});

	it('carries an unknown count as null rather than as zero', () => {
		// Zero is the wrong stand-in because it is plausible: a seller whose
		// templates could not be read would see what a seller with none sees.
		expect(tabsOf({ templates: null, mappings: 3 }).map((tab) => tab.count)).toEqual([null, 3]);
	});

	it('still counts a read that succeeded and found nothing', () => {
		expect(tabsOf({ templates: 0, mappings: 0 }).map((tab) => tab.count)).toEqual([0, 0]);
	});
});

describe('the refusal the domain already enforces', () => {
	it('never offers a licence override, which the model and a CHECK both refuse', () => {
		expect(OVERRIDABLE.licence).toBe(false);
	});
});
