// The console's half of maintenance mode. `tam-server` already answers every
// page with the maintenance page for anyone who is not an operator, so this is
// what covers the navigations that never reach it: a seller who signs in on
// `/login` and is taken into the console without a page load. Pure, so it
// tests without a component.

import type { SeasonName, SiteView } from '$lib/api';

/** The console pages that stay usable while maintenance is on: signing in and
 *  resetting a password, so an operator can get in to turn it off, and the
 *  status page the maintenance notice points at. The same list `tam-server`
 *  keeps in `serving::MAINTENANCE_OPEN`. */
export const MAINTENANCE_OPEN_ROUTES: readonly string[] = [
	'/login',
	'/reset',
	'/reset/confirm',
	'/status'
];

export type SiteGate = 'open' | 'maintenance';

export interface SiteGateInput {
	/** The switches, or null where they could not be read. */
	site: SiteView | null;
	/** Whether the signed-in human is a platform operator. */
	operator: boolean;
	pathname: string;
}

/**
 * Whether this page is the console or the maintenance notice.
 *
 * Unreadable switches open the console: maintenance mode is a courtesy, and a
 * failed read must not lock every seller out. An operator is never gated, and
 * neither is a sign-in page, so the person who can turn maintenance off can
 * always reach the switch.
 */
export function siteGate({ site, operator, pathname }: SiteGateInput): SiteGate {
	if (site === null || !site.maintenance.on || operator) {
		return 'open';
	}
	const path = pathname.length > 1 ? pathname.replace(/\/+$/, '') : pathname;
	return MAINTENANCE_OPEN_ROUTES.includes(path) ? 'open' : 'maintenance';
}

/** A seasonal theme that can show: every name but `none`. */
export type Season = Exclude<SeasonName, 'none'>;

/** Every theme, in the order the admin picker offers them: the holidays
 *  through the calendar year, then the four seasons. `when` is a reminder of
 *  when it usually runs, not a rule; the operator sets the dates. Each has a
 *  folder of stickers under the landing's `public/seasons/`, its rules in
 *  `styles/season.css`, and a mark at `web/static/seasons/<name>.svg`. */
export const SEASONS: readonly { name: Season; label: string; when: string }[] = [
	{ name: 'new-year', label: 'New Year', when: 'Around 1 January' },
	{ name: 'valentines', label: 'Valentine’s Day', when: 'Early February' },
	{ name: 'st-patricks', label: 'St Patrick’s Day', when: 'Around 17 March' },
	{ name: 'april-fools', label: 'April Fools’ Day', when: '1 April' },
	{ name: 'easter', label: 'Easter', when: 'March or April' },
	{ name: 'matariki', label: 'Matariki', when: 'June or July' },
	{ name: 'fourth-of-july', label: 'Fourth of July', when: 'Around 4 July' },
	{ name: 'back-to-school', label: 'Back to school', when: 'Before a school year starts' },
	{ name: 'halloween', label: 'Halloween', when: 'October' },
	{ name: 'guy-fawkes', label: 'Guy Fawkes', when: 'Around 5 November' },
	{ name: 'thanksgiving', label: 'Thanksgiving', when: 'October or November' },
	{ name: 'christmas', label: 'Christmas', when: 'December' },
	{ name: 'summer', label: 'Summer', when: 'A season' },
	{ name: 'autumn', label: 'Autumn', when: 'A season' },
	{ name: 'winter', label: 'Winter', when: 'A season' },
	{ name: 'spring', label: 'Spring', when: 'A season' }
];

/** The season the console's top bar marks, or null outside one. */
export function activeSeason(site: SiteView | null): Season | null {
	if (site === null || !site.theme.active || site.theme.name === 'none') {
		return null;
	}
	return site.theme.name;
}
