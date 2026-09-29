import { existsSync, readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { SiteView } from './api';
import { activeSeason, SEASONS, siteGate } from './site';

const REPO = new URL('../../../', import.meta.url).pathname;

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

	it('marks a theme whose name is more than one word under its wire spelling', () => {
		expect(activeSeason(site(false, { name: 'new-year', active: true }))).toBe('new-year');
		expect(activeSeason(site(false, { name: 'st-patricks', active: true }))).toBe('st-patricks');
	});
});

describe('the seasonal themes', () => {
	it('offers each of the sixteen themes once, and never None as a theme', () => {
		const names = SEASONS.map((entry) => entry.name);
		expect(new Set(names).size).toBe(16);
		expect(names).not.toContain('none');
	});

	// A theme the admin page offers but the landing page or the console does
	// not know saves fine and then shows nothing, so every place that spells
	// the names is held to this list.
	it('has a console mark, landing rules, stickers and a preview for every theme it offers', () => {
		const css = readFileSync(`${REPO}apps/landing/src/styles/season.css`, 'utf8');
		const base = readFileSync(`${REPO}apps/landing/src/layouts/Base.astro`, 'utf8');
		const missing = SEASONS.flatMap(({ name }) =>
			[
				existsSync(`${REPO}web/static/seasons/${name}.svg`) ? null : `${name}: console mark`,
				css.includes(`[data-season='${name}']`) ? null : `${name}: season.css rules`,
				existsSync(`${REPO}apps/landing/public/seasons/${name}`) ? null : `${name}: stickers`,
				base.includes(`'${name}'`) ? null : `${name}: ?season= preview in Base.astro`
			].filter((gap) => gap !== null)
		);
		expect(missing).toEqual([]);
	});
});
