// The two tabs this page carries, and the sentences a test pins.
//
// The sentences below are held as constants rather than written into the
// markup so that a test reads them and the lane fails when one is reworded by
// accident. No specification fixes their wording; what `decisions.md` fixes is
// the licence rule the refusal states, which `OVERRIDABLE.licence === false`
// is asserted beside. `docs/releases/0.3.0.md` quotes the older wording as a
// published record of that release and is deliberately not kept in step.

import type { Tab } from '$lib/TabBar.svelte';

export type TabId = 'templates' | 'mapping';

export const TEMPLATES: TabId = 'templates';
export const MAPPING: TabId = 'mapping';

/** The two tabs, counted from what each list actually holds.
 *
 *  Resource templates first, and where the page lands: picking, writing and
 *  applying a template is one guided flow on that tab.
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
			id: TEMPLATES,
			label: 'Resource templates',
			count: counts.templates,
			hint: 'Pick a template, see what it sets, apply it.',
			icon: 'layout-template'
		},
		{
			id: MAPPING,
			label: 'Marketplace words',
			count: counts.mappings,
			hint: 'Choose how your words show on each marketplace.',
			icon: 'sliders-horizontal'
		}
	];
}

/** Stated on the mapping tab rather than discovered by a control that always
 *  errors: `ProjectionOverride::new` and a database CHECK both refuse a
 *  licence override. */
export const LICENCE_REFUSAL =
	'You always choose the licence yourself. We never pick it for you.';
