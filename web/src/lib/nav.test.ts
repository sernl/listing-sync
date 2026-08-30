import { describe, expect, it } from 'vitest';
import {
	NAV_GROUPS,
	SETTINGS_ITEM,
	breadcrumbFor,
	initialsOf,
	isCurrent,
	legacyDestination,
	searchHref
} from './nav';

const EVERY_ITEM = [...NAV_GROUPS.flatMap((group) => group.items), SETTINGS_ITEM];

describe('the sidebar', () => {
	it('names every destination once', () => {
		const paths = EVERY_ITEM.map((item) => item.href);
		expect(new Set(paths).size).toBe(paths.length);
	});

	it('gives a count chip only where an endpoint answers one', () => {
		const counted = EVERY_ITEM.filter((item) => item.count !== undefined);
		expect(counted.map((item) => item.href)).toEqual(['/queue']);
	});

	it('never marks a destination both counted and unbuilt', () => {
		expect(EVERY_ITEM.filter((item) => item.soon && item.count !== undefined)).toEqual([]);
	});
});

describe('the current destination', () => {
	it('is the dashboard only on the dashboard, because every path is under it', () => {
		expect(isCurrent('/', '/')).toBe(true);
		expect(isCurrent('/listings', '/')).toBe(false);
	});

	it('stays lit on a child path', () => {
		expect(isCurrent('/sync/9f2c', '/sync')).toBe(true);
		expect(isCurrent('/sync', '/sync')).toBe(true);
	});

	it('does not light a destination that merely shares a prefix', () => {
		expect(isCurrent('/syncing', '/sync')).toBe(false);
	});
});

describe('the breadcrumb', () => {
	it('names the page the browser is on', () => {
		expect(breadcrumbFor('/')).toBe('Dashboard');
		expect(breadcrumbFor('/listings')).toBe('Listings');
		expect(breadcrumbFor('/queue')).toBe('Reconciliation');
		expect(breadcrumbFor('/settings')).toBe('Settings');
	});

	it('names the parent of a detail page rather than the dashboard', () => {
		expect(breadcrumbFor('/sync/9f2c8a11')).toBe('Sync');
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

	it('leave every path that did not move alone', () => {
		for (const path of ['/', '/listings', '/sync', '/connections', '/jobsy', '/queue']) {
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
	it('is the unfiltered listings page for a blank query', () => {
		expect(searchHref('')).toBe('/listings');
		expect(searchHref('   ')).toBe('/listings');
	});

	it('carries the trimmed query, encoded', () => {
		expect(searchHref('  poetry unit  ')).toBe('/listings?q=poetry%20unit');
		expect(searchHref('a&b')).toBe('/listings?q=a%26b');
	});
});
