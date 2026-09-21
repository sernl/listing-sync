// The Collections pages' own view logic: the order a member list is put in by
// the two move controls, the set arithmetic the add-to-collection dialog does,
// and the few sentences the two screens say about a collection. Pure, so it
// tests without a component.
//
// The ordering lives here rather than in the component because it is what the
// server is sent: `PUT /v1/collections/{id}/members` takes the whole ordered
// set and reads a position off each index, so the array this module returns is
// the request body, and a reorder the page got wrong would be stored wrong.

import type { CollectionHead, CollectionMemberView } from '$lib/api';
import type { InventoryId } from '$lib/generated/vocab';

/** The cache entries these pages live under. Beside the pages rather than in
 *  `$lib/query`, which holds the keys several screens share: nothing outside
 *  `pages/collections` and the board's add-to-collection dialog reads either
 *  of these, and the registry exists so an invalidation and its query cannot
 *  drift. The detail key is nested under the list's so that invalidating the
 *  list by prefix moves an open collection with it. */
export const COLLECTIONS_KEY = ['collections'] as const;

export function collectionKey(id: string): readonly [string, string] {
	return ['collections', id];
}

/** The longest a name and a description may be, which the server refuses past
 *  (`crates/tam-storage/migrations`): stated here so the field says the figure
 *  rather than leaving the seller to find it by being refused. */
export const NAME_MAX_CHARS = 80;
export const DESCRIPTION_MAX_CHARS = 1000;

/** Which way a member is being moved. Named rather than a signed number,
 *  because the two controls are "Move up" and "Move down" and an offset of -1
 *  is one indirection between the button and what it does. */
export type Move = 'up' | 'down';

/**
 * The members in the order this move leaves them.
 *
 * A move at either end returns the same order rather than wrapping: the
 * controls are disabled there, and a list that silently sent the first member
 * to the bottom would be a page whose disabled control had a second meaning. A
 * product the list does not hold is the same case — the page redraws from the
 * server's answer, so a click against a stale row must not reorder somebody
 * else's rows.
 */
export function moved(
	products: readonly string[],
	product: string,
	move: Move
): string[] {
	const from = products.indexOf(product);
	if (from === -1) {
		return [...products];
	}
	const to = move === 'up' ? from - 1 : from + 1;
	if (to < 0 || to >= products.length) {
		return [...products];
	}
	const next = [...products];
	next[from] = next[to];
	next[to] = product;
	return next;
}

/**
 * The set a collection holds once these resources are added to it.
 *
 * The existing order is kept and the new resources go on the end, in the order
 * the board had them: a collection is ordered, so adding to it is an append
 * and never a re-sort. A resource already in the collection keeps the position
 * it has rather than moving to the bottom, which is what a naive concatenation
 * would do to a seller adding one resource to a collection that already holds
 * it.
 */
export function added(
	existing: readonly string[],
	chosen: readonly string[]
): string[] {
	const held = new Set(existing);
	const next = [...existing];
	for (const product of chosen) {
		if (!held.has(product)) {
			held.add(product);
			next.push(product);
		}
	}
	return next;
}

/** The marketplaces this collection's members are on, in the console's own
 *  marketplace order, without repeats. Used by the detail page's own header,
 *  where the list is served the same union per collection. */
export function marksOf(
	members: readonly CollectionMemberView[],
	order: readonly InventoryId[]
): InventoryId[] {
	const seen = new Set(members.flatMap((member) => member.inventories));
	return order.filter((inventory) => seen.has(inventory));
}

/** How many resources a collection holds, agreeing with its figure. */
export function countLine(count: number): string {
	return count === 1 ? '1 resource' : `${count} resources`;
}

/** The collections matching a search of their names and descriptions.
 *
 * The description is searched too, because a seller who wrote "everything for
 * the autumn term" and called the collection "Term 1" has put the words they
 * would search for in the second field. */
export function matching(
	collections: readonly CollectionHead[],
	search: string
): CollectionHead[] {
	const query = search.trim().toLocaleLowerCase();
	if (query.length === 0) {
		return [...collections];
	}
	return collections.filter(
		(collection) =>
			collection.name.toLocaleLowerCase().includes(query) ||
			(collection.description ?? '').toLocaleLowerCase().includes(query)
	);
}

/** Why a name cannot be saved, or null where it can.
 *
 * The trimmed name is carried rather than recovered by the caller, so the
 * value checked and the value sent are the same string: a name accepted here
 * on its trimmed length and then sent untrimmed is a name the server refuses
 * for a reason this page said was fine. */
export type NameVerdict =
	| { accepted: true; name: string }
	| { accepted: false; message: string };

export function checkName(typed: string, taken: readonly string[] = []): NameVerdict {
	const name = typed.trim();
	if (name.length === 0) {
		return { accepted: false, message: 'A collection needs a name.' };
	}
	if (name.length > NAME_MAX_CHARS) {
		return {
			accepted: false,
			message: `A name is at most ${NAME_MAX_CHARS} characters. That one is ${name.length}.`
		};
	}
	if (taken.some((held) => held.toLocaleLowerCase() === name.toLocaleLowerCase())) {
		return { accepted: false, message: 'You already have a collection with that name.' };
	}
	return { accepted: true, name };
}

/** What a collection is, said once and before one exists.
 *
 * Written here rather than in the markup because the empty state and the
 * page's own lede say the same thing, and two copies are how they come to
 * disagree. One sentence: what the three verbs do to a whole set is the
 * `labels-and-collections` guide's. */
export const WHAT_A_COLLECTION_IS = 'A collection is an ordered set of your resources.';

/** What the free plan's seller reads under the New collection control.
 *
 * `limitReason` already says the plan includes none and to upgrade; this is
 * the sentence that says what they would be for, because a refusal with no
 * stated benefit reads as a control we took away. */
export const WHY_COLLECTIONS_COST = 'Collections are on the subscription.';
