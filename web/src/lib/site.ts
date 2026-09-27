// The console's half of maintenance mode. `tam-server` already answers every
// page with the maintenance page for anyone who is not an operator, so this is
// what covers the navigations that never reach it: a seller who signs in on
// `/login` and is taken into the console without a page load. Pure, so it
// tests without a component.

import type { SiteView } from '$lib/api';

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

/** The season the console's top bar marks, or null outside one. */
export function activeSeason(site: SiteView | null): 'halloween' | 'christmas' | null {
	if (site === null || !site.theme.active || site.theme.name === 'none') {
		return null;
	}
	return site.theme.name;
}
