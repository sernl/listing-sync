import { describe, expect, it } from 'vitest';
import {
	ADMIN_SECTION,
	ALL_DESTINATIONS,
	CREATE_TAB,
	HOME_ITEM,
	OPEN_QUESTIONS_ITEM,
	PHONE_BAR,
	PUBLIC_ROUTES,
	SECTIONS,
	accountTile,
	currentDestination,
	initialsOf,
	isCurrent,
	legacyDestination,
	searchHref,
	sectionFor,
	signedOutView
} from './nav';


describe('the navigation model', () => {
	it('lands each rail glyph on a destination the model names', () => {
		const named = new Set(ALL_DESTINATIONS.map((item) => item.href));
		for (const section of [...SECTIONS, ADMIN_SECTION]) {
			expect(named.has(section.href)).toBe(true);
		}
	});

	it('keeps an import batch report in the Import section', () => {
		expect(sectionFor('/imports/b-4')?.id).toBe('import');
	});

	it('names every destination once', () => {
		const paths = ALL_DESTINATIONS.map((item) => item.href);
		expect(new Set(paths).size).toBe(paths.length);
	});
});


describe('the phone bar', () => {
	it('puts the create cell at the centre of an odd number of cells', () => {
		expect(PHONE_BAR.length % 2).toBe(1);
		expect(PHONE_BAR[(PHONE_BAR.length - 1) / 2]).toBe(CREATE_TAB);
	});

	it('marks exactly one cell as create and gives the others a destination', () => {
		expect(PHONE_BAR.filter((tab) => tab.create === true)).toEqual([CREATE_TAB]);
		for (const tab of PHONE_BAR.filter((tab) => tab.create !== true)) {
			expect(tab.href).toBeDefined();
		}
	});

	it('does not light the create action while viewing the catalogue', () => {
		expect(isCurrent('/resources', CREATE_TAB.href)).toBe(false);
	});

	// A bar cell exists to say a section's name in fewer characters than the
	// rail's word. Where the two words are the same the cell is carrying a
	// second copy of one label, which is exactly how the rail and the bar came
	// to disagree before Crosslist was renamed to Catalogue.
	it('gives no cell a short form that repeats the label it shortens', () => {
		const repeated = PHONE_BAR.filter((tab) => tab.short === tab.label).map((tab) => tab.label);
		expect(repeated).toEqual([]);
	});
});

describe('the section the rail lights', () => {
	it('is the one holding the page the browser is on', () => {
		expect(sectionFor('/resources')?.id).toBe('crosslist');
		expect(sectionFor('/analytics')?.id).toBe('crosslist');
		expect(sectionFor('/sync')?.id).toBe('automations');
		expect(sectionFor('/automations')?.id).toBe('automations');
		expect(sectionFor('/automations/sharing')?.id).toBe('automations');
		expect(sectionFor('/settings')?.id).toBe('account');
	});

	it('follows a detail page to its parent section', () => {
		expect(sectionFor('/resources/9f2c8a11')?.id).toBe('crosslist');
		expect(sectionFor('/sync/9f2c8a11')?.id).toBe('automations');
	});

	// An import's detail page sits under `/sync` in the URL because a sync
	// request carries it, but what the seller is looking at is an import.
	// A past migrate request is opened from the Migrations list, so
	// it belongs to Automations rather than to the section that owns the path
	// it happens to sit beneath (D8).
	it('follows a request detail page to Migrations rather than to Updates', () => {
		expect(sectionFor('/sync/requests/9f2c8a11')?.id).toBe('automations');
	});

	it('leaves every other path under /sync on Updates', () => {
		expect(sectionFor('/sync')?.id).toBe('automations');
		expect(sectionFor('/sync/9f2c8a11')?.id).toBe('automations');
	});

	it('reaches a section with no page list through its landing path alone', () => {
		expect(sectionFor('/marketplaces')?.id).toBe('marketplaces');
	});

	it('prefers the longest matching page, so Subscription is not Account settings', () => {
		expect(sectionFor('/settings/subscription')?.id).toBe('account');
	});

	it('offers the operator section only to an operator', () => {
		expect(sectionFor('/admin/orgs', true)?.id).toBe('admin');
		expect(sectionFor('/admin/orgs', false)).toBeNull();
	});

	it('answers null where the path belongs to no section', () => {
		expect(sectionFor('/nowhere')).toBeNull();
		expect(sectionFor(HOME_ITEM.href)).toBeNull();
		expect(sectionFor(OPEN_QUESTIONS_ITEM.href)).toBeNull();
	});
});

describe('the current destination', () => {
	it('is the home path only on the home path', () => {
		expect(isCurrent('/app', '/app')).toBe(true);
		expect(isCurrent('/resources', '/app')).toBe(false);
	});

	it('keeps Subscription off Account settings, which prefixes it', () => {
		expect(isCurrent('/settings/subscription', '/settings')).toBe(false);
		expect(isCurrent('/settings/subscription', '/settings/subscription')).toBe(true);
	});

	it('stays lit on a child path', () => {
		expect(isCurrent('/sync/9f2c', '/sync')).toBe(true);
		expect(isCurrent('/sync', '/sync')).toBe(true);
	});

	it('does not light a destination that merely shares a prefix', () => {
		expect(isCurrent('/syncing', '/sync')).toBe(false);
	});

	it('lights one operator entry at a time, not the overview beside it', () => {
		expect(isCurrent('/admin/orgs', '/admin')).toBe(false);
		expect(isCurrent('/admin/orgs', '/admin/orgs')).toBe(true);
		expect(isCurrent('/admin', '/admin')).toBe(true);
	});

	it('keeps an organisation detail under its own group entry', () => {
		expect(isCurrent('/admin/orgs/9f2c', '/admin/orgs')).toBe(true);
		expect(isCurrent('/admin/orgs/9f2c', '/admin')).toBe(false);
	});
});


describe('the destination a path belongs to', () => {
	it('is the item whose own href contains it', () => {
		expect(currentDestination('/resources')?.href).toBe('/resources');
		expect(currentDestination('/resources/9f2c8a11')?.href).toBe('/resources');
	});

	// The reason this exists rather than an `isCurrent` per item: an import's
	// detail page sits under `/sync` and belongs to Import, so asking each item
	// whether the path is under it lights Updates, or nothing.
	it('is the item that owns a prefix it does not contain', () => {
		expect(currentDestination('/sync/requests/9f2c8a11')?.href).toBe('/automations/migration');
		expect(currentDestination('/sync/requests')?.href).toBe('/automations/migration');
	});

	it('leaves every other path under /sync on Updates', () => {
		expect(currentDestination('/sync')?.href).toBe('/sync');
		expect(currentDestination('/sync/9f2c8a11')?.href).toBe('/sync');
	});

	it('answers exactly one destination, so no two items can light at once', () => {
		for (const path of ['/sync/requests/9f2c', '/sync/9f2c', '/admin/orgs/9f2c', '/settings']) {
			const owner = currentDestination(path);
			expect(owner).not.toBeNull();
			const claimants = ALL_DESTINATIONS.filter((item) => item.href === owner?.href);
			expect(claimants).toHaveLength(1);
		}
	});

	it('prefers the longer claim where two could answer', () => {
		expect(currentDestination('/admin/orgs/9f2c8a11')?.href).toBe('/admin/orgs');
		expect(currentDestination('/settings/subscription')?.href).toBe('/settings/subscription');
		expect(currentDestination('/resources/files')?.href).toBe('/resources/files');
	});

	it('answers null for a path no destination owns', () => {
		expect(currentDestination('/nowhere')).toBeNull();
	});
});


describe('the redirects from the old paths', () => {
	it('send the jobs list to sync', () => {
		expect(legacyDestination('/jobs')).toBe('/sync');
	});

	it('carry an identifier through to the same run under its new name', () => {
		expect(legacyDestination('/jobs/9f2c8a11-0000-4000-8000-000000000000')).toBe(
			'/sync/9f2c8a11-0000-4000-8000-000000000000'
		);
	});

	it('send every screen that was renamed to its new path', () => {
		expect(legacyDestination('/listings')).toBe('/resources');
		expect(legacyDestination('/listings/new')).toBe('/resources/new');
		expect(legacyDestination('/inventory')).toBe('/resources');
		expect(legacyDestination('/inventory/new')).toBe('/resources/new');
		expect(legacyDestination('/connections')).toBe('/marketplaces');
		expect(legacyDestination('/queue')).toBe('/reconciliation');
		expect(legacyDestination('/library')).toBe('/guides');
		expect(legacyDestination('/help')).toBe('/guides');
		expect(legacyDestination('/settings/devices')).toBe('/settings#machines');
	});

	it('carry an item identifier through the catalogue renames', () => {
		expect(legacyDestination('/listings/9f2c8a11-0000-4000-8000-000000000000')).toBe(
			'/resources/9f2c8a11-0000-4000-8000-000000000000'
		);
		expect(legacyDestination('/inventory/9f2c8a11-0000-4000-8000-000000000000')).toBe(
			'/resources/9f2c8a11-0000-4000-8000-000000000000'
		);
	});

	// The catalogue has been renamed twice, so the table can accumulate a chain:
	// `/listings` pointed at `/inventory`, which now points at `/resources`. A
	// bookmark from the first naming would then need two round trips, and a
	// browser that refuses the second lands the seller nowhere.
	it('sends a twice-renamed path to its destination in one hop', () => {
		for (const from of ['/listings', '/inventory']) {
			const to = legacyDestination(from);
			expect(to).toBe('/resources');
			expect(legacyDestination(to ?? '')).toBeNull();
		}
	});


	it('leaves the catalogue itself alone, now that it owns the word', () => {
		expect(legacyDestination('/resources')).toBeNull();
		expect(legacyDestination('/resources/new')).toBeNull();
	});

	it('leave every path that did not move alone', () => {
		for (const path of ['/app', '/resources', '/sync', '/marketplaces', '/jobsy', '/reconciliation']) {
			expect(legacyDestination(path)).toBeNull();
		}
	});
});

describe('the account initials', () => {
	it('take the first letter of the first two words', () => {
		expect(initialsOf('Sunrise Teaching Co')).toBe('ST');
		expect(initialsOf('Willow')).toBe('W');
	});

	it('skip punctuation and read the letter a word actually starts with', () => {
		expect(initialsOf('  "olive"  branch ')).toBe('OB');
		expect(initialsOf('École Élan')).toBe('ÉÉ');
	});

	it('are empty rather than guessed when the name carries no letter', () => {
		expect(initialsOf('   ')).toBe('');
		expect(initialsOf('—')).toBe('');
		expect(initialsOf(undefined)).toBe('');
		expect(initialsOf(null)).toBe('');
	});
});

describe('the search destination', () => {
	it('is the unfiltered catalogue board for a blank query', () => {
		expect(searchHref('')).toBe('/resources');
		expect(searchHref('   ')).toBe('/resources');
	});

	it('carries the trimmed query, encoded', () => {
		expect(searchHref('  poetry unit  ')).toBe('/resources?q=poetry%20unit');
		expect(searchHref('a&b')).toBe('/resources?q=a%26b');
	});
});

describe('what a signed-out browser is shown', () => {
	it('renders a public page as itself', () => {
		for (const route of PUBLIC_ROUTES) {
			expect(signedOutView(route), route).toBe('public');
		}
	});

	it('stands in front of a console page instead of mounting it', () => {
		// The defect: the layout branched on the session alone, so a console
		// page's markup mounted and fired its queries for a frame before the
		// redirect landed. An implementation that answered `public` here would
		// put that frame back.
		for (const route of ['/resources', '/marketplaces', '/settings', '/admin', '/app']) {
			expect(signedOutView(route), route).toBe('redirecting');
		}
	});

	it('treats the root as a console address', () => {
		// `/` is not in the public list and must not become public by being
		// short: it is the console's own home.
		expect(signedOutView('/')).toBe('redirecting');
	});

	it('does not make a path public for sitting under a public one', () => {
		// Whole paths rather than prefixes. A prefix match would hand
		// `/status/anything` and `/login/anything` out unauthenticated.
		expect(signedOutView('/status/internals')).toBe('redirecting');
		expect(signedOutView('/login/callback')).toBe('redirecting');
	});

	it('names the confirm step separately, which is why it is reachable', () => {
		// `/reset/confirm` is a distinct entry precisely because the match is
		// exact; dropping it would send a seller following a reset link to the
		// sign-in screen instead.
		expect(signedOutView('/reset/confirm')).toBe('public');
	});

	it('offers no console destination without a session', () => {
		// The two lists are kept apart on purpose: a destination the rail
		// carries is a destination that needs a session, and a public route
		// appearing in both would be reachable from a signed-in shell that has
		// no card for it.
		const publicSet = new Set(PUBLIC_ROUTES);
		const overlap = ALL_DESTINATIONS.filter((item) => publicSet.has(item.href)).map(
			(item) => item.href
		);
		expect(overlap).toEqual(['/status']);
	});
});

describe('the account tile', () => {
	const SRC = '/v1/profile/avatar?v=3fa405a8';

	it('draws the picture where one is set, whatever the organisation is called', () => {
		expect(accountTile(SRC, 'Sunrise Teaching Co')).toEqual({ kind: 'picture', src: SRC });
		expect(accountTile(SRC, undefined)).toEqual({ kind: 'picture', src: SRC });
	});

	it('falls back to the initials where there is no picture', () => {
		expect(accountTile(null, 'Sunrise Teaching Co')).toEqual({ kind: 'initials', text: 'ST' });
	});

	it('is the section glyph while neither is known', () => {
		expect(accountTile(null, undefined)).toEqual({ kind: 'glyph' });
		expect(accountTile(null, '—')).toEqual({ kind: 'glyph' });
	});
});
