import { describe, expect, it } from 'vitest';
import { EMPTY_BODY, EMPTY_HEADING, LICENCE_REFUSAL, MAPPING, RESOURCE, tabsOf } from './tabs';
import { OVERRIDABLE } from '$lib/templates';

describe('the two tabs', () => {
	it('names the mapping tab first, which is where the page lands', () => {
		const tabs = tabsOf(3, 0);
		expect(tabs.map((tab) => tab.id)).toEqual([MAPPING, RESOURCE]);
		expect(tabs.map((tab) => tab.label)).toEqual(['Marketplace words', 'New resource']);
	});

	it('counts each tab from its own list, so neither reports the other', () => {
		expect(tabsOf(3, 5).map((tab) => tab.count)).toEqual([3, 5]);
	});

	it('carries an unknown count as null rather than as zero', () => {
		// Zero is the wrong stand-in because it is plausible: a seller whose
		// templates could not be read would see what a seller with none sees.
		expect(tabsOf(3, null).map((tab) => tab.count)).toEqual([3, null]);
	});

	it('still counts a read that succeeded and found nothing', () => {
		expect(tabsOf(0, 0).map((tab) => tab.count)).toEqual([0, 0]);
	});
});

describe('the wording the specification fixes', () => {
	it('states the licence refusal the domain already enforces', () => {
		expect(OVERRIDABLE.licence).toBe(false);
		expect(LICENCE_REFUSAL).toContain('We never pick a marketplace’s licence for you');
	});

	it('keeps the empty state exactly as specified', () => {
		expect(EMPTY_HEADING).toBe(
			"Save a resource's details as a starting point for the next one."
		);
		expect(EMPTY_BODY).toBe(
			'A template fills in the things you set the same way every time: subject, year levels, licence and price.'
		);
	});
});
