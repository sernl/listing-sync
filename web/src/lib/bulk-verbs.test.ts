import { describe, expect, it } from 'vitest';
import { BULK_ACTIONS, BULK_VERBS, unavailable } from './bulk-verbs';

describe('the bulk verbs', () => {
	it('are the five founder decision Q3 named, and no more', () => {
		expect([...BULK_VERBS].sort()).toEqual([
			'cross_list',
			'delete',
			'edit',
			'labels',
			'mark_listed'
		]);
	});

	it('names no delist-and-relist under any spelling, which is a standing refusal', () => {
		const words = BULK_ACTIONS.map((action) => `${action.verb} ${action.label}`.toLowerCase());
		for (const word of words) {
			expect(word).not.toContain('relist');
			expect(word).not.toContain('delist');
		}
	});

	it('offers the four the API already serves', () => {
		const built = BULK_ACTIONS.filter((action) => action.missing === null);
		expect(built.map((action) => action.verb)).toEqual([
			'cross_list',
			'mark_listed',
			'labels',
			'delete'
		]);
	});

	it('gives every verb it cannot run a reason naming what is missing', () => {
		for (const action of unavailable()) {
			expect(action.missing).toBeTruthy();
			expect(action.missing?.length ?? 0).toBeGreaterThan(30);
		}
	});

	it('leaves only the verb whose gap is a screen rather than an endpoint', () => {
		const byVerb = new Map(BULK_ACTIONS.map((action) => [action.verb, action]));
		expect(byVerb.get('labels')?.missing).toBeNull();
		expect(byVerb.get('mark_listed')?.missing).toBeNull();
		expect(unavailable().map((action) => action.verb)).toEqual(['edit']);
	});

	it('does not claim a missing endpoint for the verb whose endpoint exists', () => {
		const edit = BULK_ACTIONS.find((action) => action.verb === 'edit');
		expect(edit?.missing).toContain('PATCH /v1/products/{id}');
		expect(edit?.missing).toContain('screen');
	});

	it('gives every verb a label and a control to hang it on', () => {
		expect(BULK_ACTIONS).toHaveLength(BULK_VERBS.length);
		for (const action of BULK_ACTIONS) {
			expect(action.label.length).toBeGreaterThan(0);
		}
	});
});
