import { describe, expect, it } from 'vitest';
import type { Capabilities, CollectionHead, CollectionMemberView, EntitlementUsage } from '$lib/api';
import { limitReason } from '$lib/entitlement';
import { PLANS } from '$lib/generated/plans';
import type { InventoryId, Plan } from '$lib/generated/vocab';
import { INVENTORY_ORDER } from '$lib/listings-view';
import {
	DESCRIPTION_MAX_CHARS,
	NAME_MAX_CHARS,
	added,
	checkName,
	countLine,
	marksOf,
	matching,
	moved
} from './collections';

function caps(plan: Plan): Capabilities {
	const row = PLANS.find((entry) => entry.id === plan);
	if (row === undefined) {
		throw new Error(`no such plan: ${plan}`);
	}
	return row.capabilities;
}

function usage(over: Partial<EntitlementUsage> = {}): EntitlementUsage {
	return {
		resources: 0,
		marketplaces: 0,
		templates: 0,
		collections: 0,
		labels: 0,
		devices: 0,
		...over
	};
}

function member(product: string, inventories: InventoryId[] = []): CollectionMemberView {
	return { product, title: product.toUpperCase(), position: 0, inventories };
}

function head(over: Partial<CollectionHead> = {}): CollectionHead {
	return {
		id: 'c-1',
		name: 'Autumn term',
		description: null,
		count: 0,
		inventories: [],
		updated_at: 0,
		...over
	};
}

describe('moving a member', () => {
	const held = ['a', 'b', 'c'];

	it('swaps it with the one above', () => {
		expect(moved(held, 'b', 'up')).toEqual(['b', 'a', 'c']);
	});

	it('swaps it with the one below', () => {
		expect(moved(held, 'b', 'down')).toEqual(['a', 'c', 'b']);
	});

	// The order sent is the order stored, so a move off either end must not
	// wrap: the seller's first member appearing last is a write they cannot
	// see the cause of.
	it('leaves the order alone at either end', () => {
		expect(moved(held, 'a', 'up')).toEqual(held);
		expect(moved(held, 'c', 'down')).toEqual(held);
	});

	it('leaves the order alone for a member it does not hold', () => {
		expect(moved(held, 'z', 'up')).toEqual(held);
	});

	it('answers a new array, so the held order is never mutated in place', () => {
		const before = [...held];
		moved(held, 'b', 'up');
		expect(held).toEqual(before);
	});
});

describe('adding resources to a collection', () => {
	it('appends them in the order they were chosen', () => {
		expect(added(['a'], ['b', 'c'])).toEqual(['a', 'b', 'c']);
	});

	// The whole point of the arithmetic: `PUT members` replaces the set, so a
	// concatenation would both duplicate the member and move it to the bottom.
	it('leaves a resource it already holds where it is', () => {
		expect(added(['a', 'b', 'c'], ['c', 'd'])).toEqual(['a', 'b', 'c', 'd']);
	});

	it('adds nothing for an empty choice', () => {
		expect(added(['a', 'b'], [])).toEqual(['a', 'b']);
	});

	it('drops repeats within the choice itself', () => {
		expect(added([], ['a', 'a', 'b'])).toEqual(['a', 'b']);
	});
});

describe('the marks a collection shows', () => {
	it('answers each marketplace once, in the console order', () => {
		expect(
			marksOf([member('a', ['Tes', 'Tpt']), member('b', ['Tes'])], INVENTORY_ORDER)
		).toEqual(['Tpt', 'Tes']);
	});

	it('answers none for members on nothing', () => {
		expect(marksOf([member('a')], INVENTORY_ORDER)).toEqual([]);
	});
});

describe('the count line', () => {
	it('agrees with its figure', () => {
		expect(countLine(0)).toBe('0 resources');
		expect(countLine(1)).toBe('1 resource');
		expect(countLine(12)).toBe('12 resources');
	});
});

describe('searching the collections', () => {
	const all = [
		head({ id: 'c-1', name: 'Term 1', description: 'everything for the autumn term' }),
		head({ id: 'c-2', name: 'Fractions unit', description: null })
	];

	it('finds a name', () => {
		expect(matching(all, 'fractions').map((one) => one.id)).toEqual(['c-2']);
	});

	// A seller who called a collection "Term 1" and wrote what it is in the
	// description searches for the words they wrote, not the ones they filed
	// it under.
	it('finds a description', () => {
		expect(matching(all, 'autumn').map((one) => one.id)).toEqual(['c-1']);
	});

	it('answers everything for a blank search', () => {
		expect(matching(all, '   ')).toHaveLength(2);
	});
});

describe('checking a name', () => {
	it('carries the trimmed name, which is what is sent', () => {
		expect(checkName('  Autumn term  ')).toEqual({ accepted: true, name: 'Autumn term' });
	});

	it('refuses a name that is only spaces', () => {
		expect(checkName('   ').accepted).toBe(false);
	});

	it('refuses one past the stored length, stating both figures', () => {
		const verdict = checkName('x'.repeat(NAME_MAX_CHARS + 1));
		expect(verdict.accepted).toBe(false);
		expect(verdict.accepted === false && verdict.message).toContain(String(NAME_MAX_CHARS));
		expect(verdict.accepted === false && verdict.message).toContain(
			String(NAME_MAX_CHARS + 1)
		);
	});

	it('accepts one exactly at the stored length', () => {
		expect(checkName('x'.repeat(NAME_MAX_CHARS)).accepted).toBe(true);
	});

	// The server's unique index is on the lowered name, so a second "autumn
	// term" is refused there; saying so before the form is sent is the whole
	// reason this check exists.
	it('refuses a name already taken, whatever its case', () => {
		expect(checkName('AUTUMN TERM', ['Autumn term']).accepted).toBe(false);
	});

	it('states the description ceiling the server refuses past', () => {
		expect(DESCRIPTION_MAX_CHARS).toBe(1000);
	});
});

describe('the collections allowance', () => {
	// The free plan includes none, which is the arm the New collection control
	// is drawn disabled by: the sentence has to name the figure and offer the
	// way forward, because a greyed control with neither reads as a fault.
	it('refuses the free plan, saying it includes none', () => {
		const reason = limitReason(caps('free'), usage(), 'collections');
		expect(reason).toBe('Your plan includes 0 collections. Upgrade to add more.');
	});

	it('lets a subscriber with room through', () => {
		expect(limitReason(caps('subscriber'), usage({ collections: 3 }), 'collections')).toBe(
			null
		);
	});

	it('refuses a subscriber who has filled it, counting collections not labels', () => {
		const full = caps('subscriber').collections_max;
		expect(
			limitReason(caps('subscriber'), usage({ collections: full, labels: 0 }), 'collections')
		).not.toBe(null);
	});
});
