// The two tabs this page carries, and the sentences a test pins.
//
// The sentences below are held as constants rather than written into the
// markup so that a test reads them and the lane fails when one is reworded by
// accident. No specification fixes their wording; what `decisions.md` fixes is
// the licence rule the refusal states, which `OVERRIDABLE.licence === false`
// is asserted beside. `docs/releases/0.3.0.md` quotes the older wording as a
// published record of that release and is deliberately not kept in step.

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
			label: 'Marketplace words',
			count: mappings,
			hint: 'How your own words land on each marketplace.'
		},
		{
			id: RESOURCE,
			label: 'New resource',
			count: templates,
			hint: 'What a new resource starts out with.'
		}
	];
}

/** Stated on the mapping tab rather than discovered by a control that always
 *  errors: `ProjectionOverride::new` and a database CHECK both refuse a
 *  licence override. */
export const LICENCE_REFUSAL =
	'We never pick a marketplace’s licence for you: a licence has legal effect, so that choice stays yours.';

export const EMPTY_HEADING = "Save a resource's details as a starting point for the next one.";

export const EMPTY_BODY =
	'A template fills in the things you set the same way every time: subject, year levels, licence and price.';
