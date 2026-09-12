import { describe, expect, it } from 'vitest';
import { BULK_ACTIONS, BULK_VERBS, unavailable } from './bulk-verbs';

describe('the bulk verbs', () => {
	it('are the five founder decision Q3 named, the migration hand-off and the two set verbs', () => {
		expect([...BULK_VERBS].sort()).toEqual([
			'add_to_collection',
			'apply_template',
			'cross_list',
			'delete',
			'edit',
			'labels',
			'mark_listed',
			'move'
		]);
	});

	it('names no delist-and-relist under any spelling, which is a standing refusal', () => {
		const words = BULK_ACTIONS.map((action) => `${action.verb} ${action.label}`.toLowerCase());
		for (const word of words) {
			expect(word).not.toContain('relist');
			expect(word).not.toContain('delist');
		}
	});

	it('offers everything the API already serves', () => {
		const built = BULK_ACTIONS.filter((action) => action.missing === null);
		expect(built.map((action) => action.verb)).toEqual([
			'cross_list',
			'mark_listed',
			'move',
			'labels',
			'add_to_collection',
			'apply_template',
			'delete'
		]);
	});

	// Copy and Move are one control here rather than two, because which of them
	// the seller wants is a decision they take on the Migrations page beside the
	// preview of what each would do, not blind on a bulk bar.
	it('offers copying and moving under one verb, worded as both', () => {
		const move = BULK_ACTIONS.find((action) => action.verb === 'move');
		expect(move?.missing).toBeNull();
		expect(move?.label.toLowerCase()).toContain('copy');
		expect(move?.label.toLowerCase()).toContain('move');
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

	it('says the gap is a screen rather than an endpoint, in words a seller reads', () => {
		const edit = BULK_ACTIONS.find((action) => action.verb === 'edit');
		expect(edit?.missing).toContain('screen');
		// The sentence is read on a phone, where it is the only touch-reachable
		// statement of the refusal -- the menu item carries the same words in a
		// `title`, which a thumb cannot open. A route path is a fact for us, so
		// none of the reasons may name one.
		for (const action of BULK_ACTIONS) {
			expect(action.missing ?? '').not.toMatch(/\/v1\//);
			expect(action.missing ?? '').not.toMatch(/\b(GET|POST|PATCH|PUT|DELETE)\b/);
		}
	});

	it('gives every verb a label and a control to hang it on', () => {
		expect(BULK_ACTIONS).toHaveLength(BULK_VERBS.length);
		for (const action of BULK_ACTIONS) {
			expect(action.label.length).toBeGreaterThan(0);
		}
	});
});
