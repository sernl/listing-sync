import { describe, expect, it } from 'vitest';
import { MAPPING, NEW, SAVED, tabsOf } from './tabs';
import { OVERRIDABLE } from '$lib/templates';

describe('the three tabs', () => {
	it('names the editor first, which is where the page lands', () => {
		const tabs = tabsOf({ templates: 0, mappings: 3 });
		expect(tabs.map((tab) => tab.id)).toEqual([NEW, SAVED, MAPPING]);
	});

	it('counts each tab from its own list, so neither reports the other', () => {
		expect(tabsOf({ templates: 5, mappings: 3 }).map((tab) => tab.count)).toEqual([null, 5, 3]);
	});

	it('carries an unknown count as null rather than as zero', () => {
		// Zero is the wrong stand-in because it is plausible: a seller whose
		// templates could not be read would see what a seller with none sees.
		expect(tabsOf({ templates: null, mappings: 3 }).map((tab) => tab.count)).toEqual([
			null,
			null,
			3
		]);
	});

	it('still counts a read that succeeded and found nothing', () => {
		expect(tabsOf({ templates: 0, mappings: 0 }).map((tab) => tab.count)).toEqual([null, 0, 0]);
	});
});

describe('the refusal the domain already enforces', () => {
	it('never offers a licence override, which the model and a CHECK both refuse', () => {
		expect(OVERRIDABLE.licence).toBe(false);
	});
});
