import { describe, expect, it } from 'vitest';
import type { SiteView } from './api';
import { activeSeason, siteGate } from './site';

function site(on: boolean, theme: Partial<SiteView['theme']> = {}): SiteView {
	return {
		maintenance: { on, message: on ? 'Back by 3pm UTC.' : null },
		theme: { name: 'none', from: null, until: null, active: false, ...theme },
		banner: null
	};
}

describe('the maintenance gate', () => {
	it('shows a seller the notice on every console page while maintenance is on', () => {
		for (const pathname of ['/', '/app', '/resources', '/settings/billing', '/admin/site']) {
			expect(siteGate({ site: site(true), operator: false, pathname })).toBe('maintenance');
		}
	});

	it('lets an operator use the console, so they can turn maintenance off', () => {
		expect(siteGate({ site: site(true), operator: true, pathname: '/admin/site' })).toBe('open');
	});

	it('keeps sign-in, password reset and status reachable, with or without a trailing slash', () => {
		for (const pathname of ['/login', '/login/', '/reset', '/reset/confirm', '/status']) {
			expect(siteGate({ site: site(true), operator: false, pathname })).toBe('open');
		}
	});

	it('gates a page that only begins like an open one', () => {
		expect(siteGate({ site: site(true), operator: false, pathname: '/login-help' })).toBe(
			'maintenance'
		);
	});

	it('opens the console when maintenance is off or the switches could not be read', () => {
		expect(siteGate({ site: site(false), operator: false, pathname: '/app' })).toBe('open');
		expect(siteGate({ site: null, operator: false, pathname: '/app' })).toBe('open');
	});
});

describe('the season mark', () => {
	it('marks the season only while the server says it is active', () => {
		expect(activeSeason(site(false, { name: 'halloween', active: true }))).toBe('halloween');
		expect(activeSeason(site(false, { name: 'halloween', active: false }))).toBeNull();
		expect(activeSeason(site(false, { name: 'none', active: true }))).toBeNull();
		expect(activeSeason(null)).toBeNull();
	});
});
