// The two tabs this page carries and the wording the specification fixes.
//
// The sentences below are held as constants rather than written into the
// markup because they are the specification's own words, and a test that
// reads them fails the lane when one is reworded by accident.

import type { Tab } from '$lib/TabBar.svelte';

export type TabId = 'mapping' | 'resource';

export const MAPPING: TabId = 'mapping';
export const RESOURCE: TabId = 'resource';

/** The two tabs, counted from what each list actually holds.
 *
 *  A count is null until a read has produced one, which covers both the first
 *  read and one that failed: a tab that reported zero in either case would say
 *  the seller has none, and only one of those two sellers would be being told
 *  the truth. A read that succeeded and returned nothing still counts zero. */
export function tabsOf(mappings: number | null, templates: number | null): Tab[] {
	return [
		{
			id: MAPPING,
			label: 'Marketplace mapping',
			count: mappings,
			hint: 'How your own words land on each marketplace.'
		},
		{
			id: RESOURCE,
			label: 'New resource',
			count: templates,
			hint: 'What a new resource starts out filled in with.'
		}
	];
}

/** Stated on the mapping tab rather than discovered by a control that always
 *  errors: `ProjectionOverride::new` and a database CHECK both refuse a
 *  licence override. */
export const LICENCE_REFUSAL =
	"Licence is never mapped for you. A licence has legal effect, so every marketplace's licence is your own decision.";

export const EMPTY_HEADING = "Save a resource's details as a starting point for the next one.";

export const EMPTY_BODY =
	'A template keeps the fields you fill the same way every time — subject, year levels, licence, price — so a new resource starts most of the way there.';
