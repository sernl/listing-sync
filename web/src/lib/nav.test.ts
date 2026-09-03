import { describe, expect, it } from 'vitest';
import {
	ADMIN_GROUP,
	LEGACY_REDIRECTS,
	MOBILE_TABS,
	NAV_GROUPS,
	SETTINGS_ITEM,
	breadcrumbFor,
	initialsOf,
	isCurrent,
	legacyDestination,
	searchHref
} from './nav';

const EVERY_ITEM = [
	...NAV_GROUPS.flatMap((group) => group.items),
	...ADMIN_GROUP.items,
	SETTINGS_ITEM
];

describe('the sidebar', () => {
	it('names every destination once', () => {
		const paths = EVERY_ITEM.map((item) => item.href);
		expect(new Set(paths).size).toBe(paths.length);
	});

	it('gives a count chip only where an endpoint answers one', () => {
		const counted = EVERY_ITEM.filter((item) => item.count !== undefined);
		expect(counted.map((item) => item.href)).toEqual(['/reconciliation']);
	});

	it('never marks a destination both counted and unbuilt', () => {
		expect(EVERY_ITEM.filter((item) => item.soon && item.count !== undefined)).toEqual([]);
	});
});

describe('the phone tab bar', () => {
	it('carries the five destinations the small viewport navigates by', () => {
		expect(MOBILE_TABS.map((tab) => tab.href)).toEqual([
			'/inventory',
			'/marketplaces',
			'/sync',
			'/analytics',
			'/settings'
		]);
	});

	it('names each tab in a word that fits a tab', () => {
		expect(MOBILE_TABS.map((tab) => tab.label)).toEqual([
			'Inventory',
			'Marketplaces',
			'Sync',
			'Analytics',
			'Account'
		]);
	});

	it('points every tab at a destination the sidebar holds', () => {
		const destinations = new Set(EVERY_ITEM.map((item) => item.href));
		for (const tab of MOBILE_TABS) {
			expect(destinations.has(tab.href)).toBe(true);
		}
	});

	it('takes each glyph from that destination rather than a second copy', () => {
		for (const tab of MOBILE_TABS) {
			const destination = EVERY_ITEM.find((item) => item.href === tab.href);
			expect(tab.icon).toBe(destination?.icon);
		}
	});

	it('offers no tab to a destination that is not built', () => {
		expect(MOBILE_TABS.filter((tab) => tab.soon)).toEqual([]);
	});
});

describe('the current destination', () => {
	it('is the dashboard only on the dashboard, because every path is under it', () => {
		expect(isCurrent('/', '/')).toBe(true);
		expect(isCurrent('/inventory', '/')).toBe(false);
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

describe('the operator group', () => {
	it('is not one of the groups every seller sees', () => {
		expect(NAV_GROUPS.map((group) => group.label)).not.toContain(ADMIN_GROUP.label);
	});

	it('sends every entry into the operator subtree', () => {
		for (const item of ADMIN_GROUP.items) {
			expect(item.href.startsWith('/admin')).toBe(true);
		}
	});

	it('marks no operator destination unbuilt', () => {
		expect(ADMIN_GROUP.items.filter((item) => item.soon)).toEqual([]);
	});
});

describe('the breadcrumb', () => {
	it('names the page the browser is on', () => {
		expect(breadcrumbFor('/')).toBe('Dashboard');
		expect(breadcrumbFor('/inventory')).toBe('Inventory');
		expect(breadcrumbFor('/reconciliation')).toBe('Reconciliation');
		expect(breadcrumbFor('/settings')).toBe('Account Settings');
		expect(breadcrumbFor('/marketplaces')).toBe('Marketplaces');
	});

	it('names the parent of a detail page rather than the dashboard', () => {
		expect(breadcrumbFor('/sync/9f2c8a11')).toBe('Sync');
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
		expect(legacyDestination('/listings')).toBe('/inventory');
		expect(legacyDestination('/listings/new')).toBe('/inventory/new');
		expect(legacyDestination('/connections')).toBe('/marketplaces');
		expect(legacyDestination('/queue')).toBe('/reconciliation');
		expect(legacyDestination('/library')).toBe('/resources');
		expect(legacyDestination('/settings/devices')).toBe('/marketplaces');
	});

	it('carry an item identifier through the inventory rename', () => {
		expect(legacyDestination('/listings/9f2c8a11-0000-4000-8000-000000000000')).toBe(
			'/inventory/9f2c8a11-0000-4000-8000-000000000000'
		);
	});

	it('name a destination the sidebar still holds, so no redirect lands on nothing', () => {
		const destinations = new Set(EVERY_ITEM.map((item) => item.href));
		for (const { to } of LEGACY_REDIRECTS) {
			expect(destinations.has(to)).toBe(true);
		}
	});

	it('leave every path that did not move alone', () => {
		for (const path of ['/', '/inventory', '/sync', '/marketplaces', '/jobsy', '/reconciliation']) {
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
	it('is the unfiltered inventory board for a blank query', () => {
		expect(searchHref('')).toBe('/inventory');
		expect(searchHref('   ')).toBe('/inventory');
	});

	it('carries the trimmed query, encoded', () => {
		expect(searchHref('  poetry unit  ')).toBe('/inventory?q=poetry%20unit');
		expect(searchHref('a&b')).toBe('/inventory?q=a%26b');
	});
});
