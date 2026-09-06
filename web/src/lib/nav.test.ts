import { describe, expect, it } from 'vitest';
import {
	ADMIN_SECTION,
	ALL_DESTINATIONS,
	CREATE_TAB,
	HOME_ITEM,
	LEGACY_REDIRECTS,
	OPEN_QUESTIONS_ITEM,
	PHONE_BAR,
	SEARCH_TAB,
	SECTION_TABS,
	SECTIONS,
	breadcrumbFor,
	currentDestination,
	initialsOf,
	isCurrent,
	legacyDestination,
	searchHref,
	sectionFor
} from './nav';

// Read from the model rather than composed again here: a second composition is
// what let `/marketplaces` be reachable in the shell and absent from the list
// the redirect test checks against.
const EVERY_ITEM = ALL_DESTINATIONS;

const pagesOf = (id: string) =>
	SECTIONS.find((section) => section.id === id)?.items.map((item) => item.href);

describe('the navigation model', () => {
	it('names the three sections the founder asked for, then Account', () => {
		expect(SECTIONS.map((section) => section.label)).toEqual([
			'Crosslist',
			'Automations',
			'Marketplaces',
			'Account'
		]);
	});

	it('gives a section-primary button to Crosslist alone', () => {
		const withPrimary = SECTIONS.filter((section) => section.primary !== undefined);
		expect(withPrimary.map((section) => section.id)).toEqual(['crosslist']);
	});

	// A rail glyph lands either on one of its section's own pages, as Crosslist
	// and Account do, or on a landing page of its own, as Automations and
	// Marketplaces do. Either way the path has to be a destination the model
	// names, or the breadcrumb answers "Console" on it.
	it('lands each rail glyph on a destination the model names', () => {
		const named = new Set(ALL_DESTINATIONS.map((item) => item.href));
		for (const section of [...SECTIONS, ADMIN_SECTION]) {
			expect(named.has(section.href)).toBe(true);
		}
	});

	it('gives Automations a landing page of its own, with the sync list inside it', () => {
		const automations = SECTIONS.find((section) => section.id === 'automations');
		expect(automations?.href).toBe('/automations');
		expect(automations?.items.map((item) => item.href)).toContain('/sync');
		expect(automations?.items.some((item) => item.href === '/automations')).toBe(false);
	});

	it('leaves Marketplaces without a page list, so it renders no card', () => {
		expect(SECTIONS.find((section) => section.id === 'marketplaces')?.items).toEqual([]);
	});

	// Enumerated rather than counted: a count catches a deletion and misses a
	// substitution, and nothing else in this file reads the page lists at all.
	it('lists exactly the Crosslist pages, in order', () => {
		expect(pagesOf('crosslist')).toEqual([
			'/resources',
			'/labels',
			'/import',
			'/analytics',
			'/templates',
			'/export'
		]);
	});

	it('lists exactly the Automations pages, in order', () => {
		expect(pagesOf('automations')).toEqual([
			'/automations/sharing',
			'/automations/migration',
			'/sync'
		]);
	});

	it('lists exactly the Account pages, in order', () => {
		expect(pagesOf('account')).toEqual([
			'/settings',
			'/settings/subscription',
			'/notifications',
			'/status',
			'/guides'
		]);
	});

	// Which pages are unbuilt is asserted in `nav-routes.test.ts`, against the
	// routes themselves. A list kept here would pass while a slice finished its
	// page and left the flag on, which is the failure that actually happens.

	it('never marks the landing page of a section unbuilt', () => {
		for (const section of SECTIONS) {
			const landing = section.items.find((item) => item.href === section.href);
			expect(landing?.soon).toBeUndefined();
		}
	});

	it('names every destination once', () => {
		const paths = EVERY_ITEM.map((item) => item.href);
		expect(new Set(paths).size).toBe(paths.length);
	});

	it('keeps the home path and the queue off the rail but inside the model', () => {
		const listed = SECTIONS.flatMap((section) => section.items).map((item) => item.href);
		expect(listed).not.toContain(HOME_ITEM.href);
		expect(listed).not.toContain(OPEN_QUESTIONS_ITEM.href);
	});
});

// The rail's sections rendered as tabs, which is the navigating half of the
// phone bar rather than the bar itself: `PHONE_BAR` below drops Account from
// these and adds the create and search cells.
describe('the section tabs', () => {
	it('mirrors the rail rather than choosing its own destinations', () => {
		expect(SECTION_TABS.map((tab) => tab.href)).toEqual(SECTIONS.map((section) => section.href));
		expect(SECTION_TABS.map((tab) => tab.label)).toEqual(SECTIONS.map((section) => section.label));
	});

	it('carries all four sections the rail names, in rail order', () => {
		expect(SECTION_TABS.map((tab) => tab.href)).toEqual([
			'/resources',
			'/automations',
			'/marketplaces',
			'/settings'
		]);
	});

	it('names the home path for what it shows, not for a dashboard', () => {
		expect(HOME_ITEM.href).toBe('/app');
		expect(HOME_ITEM.label).toBe('Resources');
		const resources = SECTIONS.find((section) => section.id === 'crosslist')?.items[0];
		expect(HOME_ITEM.icon).toBe(resources?.icon);
	});

	it('takes each glyph from its section rather than a second copy', () => {
		for (const [index, tab] of SECTION_TABS.entries()) {
			expect(tab.icon).toBe(SECTIONS[index].icon);
		}
	});

	// Checked against the section's own item rather than the derived tab: the
	// tab is built from `{ href, label, icon }` and cannot carry `soon` at all,
	// so asserting it there passes whatever the model says.
	it('offers no tab to a section whose landing page is not built', () => {
		for (const tab of SECTION_TABS) {
			const landing = SECTIONS.flatMap((section) => section.items).find(
				(item) => item.href === tab.href
			);
			expect(landing?.soon).toBeUndefined();
		}
	});
});

describe('the phone bar', () => {
	it('carries five items, with the create action in the middle', () => {
		expect(PHONE_BAR.map((tab) => tab.label)).toEqual([
			'Crosslist',
			'Automations',
			'New resource',
			'Marketplaces',
			'Search'
		]);
	});

	it('names the create action exactly as the navigation card names it', () => {
		const primary = SECTIONS.find((section) => section.id === 'crosslist')?.primary;
		expect(CREATE_TAB.label).toBe(primary?.label);
		expect(CREATE_TAB.label).toBe('New resource');
	});

	it('marks exactly one cell as create and exactly one as search', () => {
		expect(PHONE_BAR.filter((tab) => tab.create === true)).toEqual([CREATE_TAB]);
		expect(PHONE_BAR.filter((tab) => tab.search === true)).toEqual([SEARCH_TAB]);
		expect(PHONE_BAR[2]).toBe(CREATE_TAB);
	});

	it('navigates only to the sections the rail states, and never twice', () => {
		const going = PHONE_BAR.filter((tab) => tab.create !== true && tab.search !== true);
		expect(going).toEqual(SECTION_TABS.filter((tab) => tab.href !== '/settings'));
		const hrefs = PHONE_BAR.map((tab) => tab.href).filter((href) => href !== undefined);
		expect(new Set(hrefs).size).toBe(hrefs.length);
	});

	// The one deliberate disagreement between the rail and the bar, pinned so
	// it cannot drift back by accident. Account stays a section and keeps its
	// rail entry; on a phone it is the avatar in the top strip instead, which
	// is where the rail already puts it -- at the foot beside Help, rather than
	// among the sections.
	it('leaves Account off the bar while keeping it a section', () => {
		expect(SECTIONS.some((section) => section.id === 'account')).toBe(true);
		expect(SECTION_TABS.some((tab) => tab.href === '/settings')).toBe(true);
		expect(PHONE_BAR.some((tab) => tab.href === '/settings')).toBe(false);
	});

	it('gives the search cell no destination, so it lights nothing', () => {
		expect(SEARCH_TAB.href).toBeUndefined();
		expect(SECTION_TABS.some((tab) => tab.label === SEARCH_TAB.label)).toBe(false);
	});

	it('opens the same screen the section-primary button opens', () => {
		const primary = SECTIONS.find((section) => section.id === 'crosslist')?.primary;
		expect(CREATE_TAB.href).toBe(primary?.href);
	});

	it('is not a navigation destination, so it lights nothing', () => {
		expect(isCurrent('/resources', CREATE_TAB.href)).toBe(false);
		expect(SECTION_TABS.some((tab) => tab.href === CREATE_TAB.href)).toBe(false);
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
	// A past migrate request is opened from the Marketplace Migration list, so
	// it belongs to Automations rather than to the section that owns the path
	// it happens to sit beneath (D8).
	it('follows a request detail page to Migration rather than to Marketplace Sync', () => {
		expect(sectionFor('/sync/requests/9f2c8a11')?.id).toBe('automations');
		expect(breadcrumbFor('/sync/requests/9f2c8a11')).toBe('Marketplace Migration');
	});

	it('leaves every other path under /sync on Marketplace Sync', () => {
		expect(sectionFor('/sync')?.id).toBe('automations');
		expect(sectionFor('/sync/9f2c8a11')?.id).toBe('automations');
		expect(breadcrumbFor('/sync/9f2c8a11')).toBe('Marketplace Sync');
	});

	it('reaches a section with no page list through its landing path alone', () => {
		expect(sectionFor('/marketplaces')?.id).toBe('marketplaces');
	});

	it('prefers the longest matching page, so Subscription is not Preferences', () => {
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

	it('keeps Subscription off Preferences, which prefixes it', () => {
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

describe('the operator section', () => {
	it('is not one of the sections every seller sees', () => {
		expect(SECTIONS.map((section) => section.id)).not.toContain(ADMIN_SECTION.id);
	});

	it('sends every entry into the operator subtree', () => {
		for (const item of ADMIN_SECTION.items) {
			expect(item.href.startsWith('/admin')).toBe(true);
		}
	});

	it('marks no operator destination unbuilt', () => {
		expect(ADMIN_SECTION.items.filter((item) => item.soon)).toEqual([]);
	});
});

describe('the destination a path belongs to', () => {
	it('is the item whose own href contains it', () => {
		expect(currentDestination('/resources')?.href).toBe('/resources');
		expect(currentDestination('/resources/9f2c8a11')?.href).toBe('/resources');
	});

	// The reason this exists rather than an `isCurrent` per item: an import's
	// detail page sits under `/sync` and belongs to Import, so asking each item
	// whether the path is under it lights Marketplace Sync, or nothing.
	it('is the item that owns a prefix it does not contain', () => {
		expect(currentDestination('/sync/requests/9f2c8a11')?.href).toBe('/automations/migration');
		expect(currentDestination('/sync/requests')?.href).toBe('/automations/migration');
	});

	it('leaves every other path under /sync on Marketplace Sync', () => {
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
	});

	it('answers null for a path no destination owns', () => {
		expect(currentDestination('/nowhere')).toBeNull();
	});
});

describe('the breadcrumb', () => {
	it('names the page the browser is on', () => {
		expect(breadcrumbFor('/app')).toBe('Resources');
		expect(breadcrumbFor('/resources')).toBe('Resources');
		expect(breadcrumbFor('/reconciliation')).toBe('Open questions');
		expect(breadcrumbFor('/settings')).toBe('Preferences');
		expect(breadcrumbFor('/marketplaces')).toBe('Marketplaces');
	});

	it('names the parent of a detail page rather than the home path', () => {
		expect(breadcrumbFor('/sync/9f2c8a11')).toBe('Marketplace Sync');
	});

	it('lets a destination claim a path that is not under its own href', () => {
		const owner = SECTIONS.flatMap((section) => section.items).find(
			(item) => item.href === '/automations/migration'
		);
		expect(owner?.owns).toEqual(['/sync/requests']);
		expect(breadcrumbFor('/sync/requests')).toBe('Marketplace Migration');
	});

	it('leaves exactly one destination owning that prefix', () => {
		const owners = SECTIONS.flatMap((section) => section.items).filter((item) =>
			item.owns?.includes('/sync/requests')
		);
		expect(owners.map((item) => item.href)).toEqual(['/automations/migration']);
	});

	it('names the operator page rather than its group', () => {
		expect(breadcrumbFor('/admin')).toBe('Overview');
		expect(breadcrumbFor('/admin/orgs')).toBe('Organisations');
		expect(breadcrumbFor('/admin/orgs/9f2c8a11')).toBe('Organisations');
		expect(breadcrumbFor('/admin/users')).toBe('Identity users');
	});

	it('falls back to the console for a path the sidebar does not name', () => {
		expect(breadcrumbFor('/nowhere')).toBe('Console');
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
		expect(legacyDestination('/settings/devices')).toBe('/marketplaces');
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

	it('name a destination the sidebar still holds, so no redirect lands on nothing', () => {
		const destinations = new Set(EVERY_ITEM.map((item) => item.href));
		for (const { to } of LEGACY_REDIRECTS) {
			expect(destinations.has(to)).toBe(true);
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
