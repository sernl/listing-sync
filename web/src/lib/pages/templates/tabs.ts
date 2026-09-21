// The three tabs this page carries, and the sentences a test pins.
//
// The sentences below are held as constants rather than written into the
// markup so that a test reads them and the lane fails when one is reworded by
// accident. No specification fixes their wording; what `decisions.md` fixes is
// the licence rule the refusal states, which `OVERRIDABLE.licence === false`
// is asserted beside. `docs/releases/0.3.0.md` quotes the older wording as a
// published record of that release and is deliberately not kept in step.

import type { Tab } from '$lib/TabBar.svelte';

export type TabId = 'new' | 'saved' | 'mapping';

export const NEW: TabId = 'new';
export const SAVED: TabId = 'saved';
export const MAPPING: TabId = 'mapping';

/** The three tabs, counted from what each list actually holds.
 *
 *  New template is first and is where the page lands: writing one is the task
 *  a seller comes here for, and its editor is open on arrival, so the landing
 *  tab is the work rather than a list to press past.
 *
 *  It carries no count because it holds no rows — an editor is one form, and a
 *  parenthesis beside it would be a figure standing for nothing.
 *
 *  A count is null until a read has produced one, which covers both the first
 *  read and one that failed: a tab that reported zero in either case would say
 *  the seller has none, and only one of those two sellers would be being told
 *  the truth. A read that succeeded and returned nothing still counts zero.
 *
 *  Named rather than positional: the two counts are both `number | null`, so a
 *  swapped pair would report each list under the other's label with nothing to
 *  catch it. */
export function tabsOf(counts: { templates: number | null; mappings: number | null }): Tab[] {
	return [
		{
			id: NEW,
			label: 'New template',
			count: null,
			hint: 'What a new resource starts out with.',
			icon: 'circle-plus'
		},
		{
			id: SAVED,
			label: 'Saved templates',
			count: counts.templates,
			hint: 'The starting points you have already saved.',
			icon: 'layout-template'
		},
		{
			id: MAPPING,
			label: 'Marketplace words',
			count: counts.mappings,
			hint: 'How your own words land on each marketplace.',
			icon: 'sliders-horizontal'
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
