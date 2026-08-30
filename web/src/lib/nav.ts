// The console shell's own model: the grouped sidebar, the breadcrumb the top
// bar shows, the paths the old information architecture used, and the two
// small renderings the account card and the search box need. Pure, so it
// tests without a component.

/** A count a nav chip may carry. Only the reconciliation queue has an
 *  endpoint that answers one cheaply and honestly, so it is the only name
 *  here: a chip with no real figure behind it is a `soon` chip instead. */
export type NavCount = 'reconciliation';

export interface NavItem {
	href: string;
	label: string;
	/** The glyph the mockup gives this destination. */
	icon: string;
	/** Marks a destination that exists as a route and not yet as a feature. */
	soon?: true;
	count?: NavCount;
}

export interface NavGroup {
	label: string;
	items: readonly NavItem[];
}

export const NAV_GROUPS: readonly NavGroup[] = [
	{
		label: 'Workspace',
		items: [
			{ href: '/', label: 'Dashboard', icon: '▦' },
			{ href: '/listings', label: 'Listings', icon: '▤' },
			{ href: '/sync', label: 'Sync', icon: '⇄' },
			{ href: '/connections', label: 'Connections', icon: '⚲' },
			{ href: '/analytics', label: 'Analytics', icon: '◔' },
			{ href: '/queue', label: 'Reconciliation', icon: '☰', count: 'reconciliation' }
		]
	},
	{
		label: 'Buyer',
		items: [{ href: '/purchases', label: 'Purchases', icon: '◨', soon: true }]
	},
	{
		label: 'Tools',
		items: [
			{ href: '/library', label: 'Library', icon: '≣', soon: true },
			{ href: '/templates', label: 'Templates', icon: '❏', soon: true },
			{ href: '/notifications', label: 'Notifications', icon: '◷', soon: true },
			{ href: '/status', label: 'Status', icon: '◉' },
			{ href: '/help', label: 'Help', icon: '?', soon: true }
		]
	}
];

/** Pinned below the groups, beside the account card. */
export const SETTINGS_ITEM: NavItem = { href: '/settings', label: 'Settings', icon: '⚙' };

const ALL_ITEMS: readonly NavItem[] = [
	...NAV_GROUPS.flatMap((group) => group.items),
	SETTINGS_ITEM
];

/** Whether a nav destination is the one the browser is on.
 *
 * A prefix match everywhere but the dashboard, so a job detail under `/sync`
 * still lights its parent; the dashboard is matched exactly, because every
 * path is under `/`. */
export function isCurrent(pathname: string, href: string): boolean {
	if (href === '/') {
		return pathname === '/';
	}
	return pathname === href || pathname.startsWith(`${href}/`);
}

/** The word after `Console /` in the top bar. The longest matching
 *  destination wins, so `/sync/<id>` reads as Sync rather than as the first
 *  nav entry that happens to prefix it. */
export function breadcrumbFor(pathname: string): string {
	let best: NavItem | null = null;
	for (const item of ALL_ITEMS) {
		if (isCurrent(pathname, item.href) && (best === null || item.href.length > best.href.length)) {
			best = item;
		}
	}
	return best?.label ?? 'Console';
}

/** Where a path from the old information architecture now lives.
 *
 * Prefixes rather than whole paths: `/jobs/<id>` has to land on the same job
 * under its new name, and a redirect table listing identifiers could not. */
export const LEGACY_REDIRECTS: readonly { from: string; to: string }[] = [
	{ from: '/jobs', to: '/sync' }
];

export function legacyDestination(pathname: string): string | null {
	for (const { from, to } of LEGACY_REDIRECTS) {
		if (pathname === from) {
			return to;
		}
		if (pathname.startsWith(`${from}/`)) {
			return to + pathname.slice(from.length);
		}
	}
	return null;
}

/** The account card's avatar: up to two initials from the organisation name.
 *
 * Empty for a name that carries no letter or digit, which the card renders as
 * an empty tile rather than as a guessed character. */
export function initialsOf(name: string | undefined | null): string {
	if (typeof name !== 'string') {
		return '';
	}
	const words = name
		.split(/\s+/)
		.map((word) => [...word].find((glyph) => /\p{L}|\p{N}/u.test(glyph)))
		.filter((glyph): glyph is string => glyph !== undefined);
	return words.slice(0, 2).join('').toUpperCase();
}

/** Where the top-bar search sends the browser. A blank query is the listings
 *  page with no filter rather than an empty `?q=`. */
export function searchHref(query: string): string {
	const trimmed = query.trim();
	return trimmed.length === 0 ? '/listings' : `/listings?q=${encodeURIComponent(trimmed)}`;
}
