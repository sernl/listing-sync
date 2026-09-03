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
			{ href: '/inventory', label: 'Inventory', icon: '▤' },
			{ href: '/marketplaces', label: 'Marketplaces', icon: '⚲' },
			{ href: '/analytics', label: 'Analytics', icon: '◔' },
			{ href: '/reconciliation', label: 'Reconciliation', icon: '☰', count: 'reconciliation' }
		]
	},
	{
		label: 'Automations',
		items: [{ href: '/sync', label: 'Sync', icon: '⇄' }]
	},
	{
		label: 'Buyer',
		items: [{ href: '/purchases', label: 'Purchases', icon: '◨', soon: true }]
	},
	{
		label: 'Crosslist',
		items: [{ href: '/templates', label: 'Templates', icon: '❏', soon: true }]
	},
	// Last, so it renders at the foot of the sidebar beside the account card:
	// Vendoo keeps its status page in the profile menu and its help centre in a
	// help menu, and none of these four is a crosslisting tool.
	{
		label: 'Help',
		items: [
			{ href: '/resources', label: 'Resources', icon: '≣', soon: true },
			{ href: '/notifications', label: 'Notifications', icon: '◷', soon: true },
			{ href: '/status', label: 'Status', icon: '◉' },
			{ href: '/help', label: 'Help', icon: '?', soon: true }
		]
	}
];

/** The operator's own group, rendered under the others and only for a human
 *  the operator probe admitted.
 *
 *  Separate from `NAV_GROUPS` rather than a flag inside it, because every
 *  entry there is unconditional: a seller sees exactly that sidebar, and a
 *  conditional item mixed into the same list is one `if` away from leaking a
 *  destination that answers 401. Its destinations still refuse a
 *  non-operator on their own — the hiding is courtesy, never the fence. */
export const ADMIN_GROUP: NavGroup = {
	label: 'Admin',
	items: [
		{ href: '/admin', label: 'Overview', icon: '◈' },
		{ href: '/admin/orgs', label: 'Organisations', icon: '⌂' },
		{ href: '/admin/health', label: 'Sync health', icon: '❤' },
		{ href: '/admin/failures', label: 'Failed writes', icon: '✕' },
		{ href: '/admin/users', label: 'Identity users', icon: '☺' },
		{ href: '/admin/impersonations', label: 'Impersonations', icon: '⧉' }
	]
};

/** Pinned below the groups, beside the account card. */
export const SETTINGS_ITEM: NavItem = { href: '/settings', label: 'Account Settings', icon: '⚙' };

const ALL_ITEMS: readonly NavItem[] = [
	...NAV_GROUPS.flatMap((group) => group.items),
	...ADMIN_GROUP.items,
	SETTINGS_ITEM
];

/** Destinations matched exactly rather than by prefix, because each one has
 *  sibling destinations of its own beneath it in the same sidebar: every path
 *  is under `/`, and every operator page is under `/admin`. A prefix match on
 *  either would light two entries at once. */
const EXACT_ONLY: readonly string[] = ['/', '/admin'];

/** Whether a nav destination is the one the browser is on.
 *
 * A prefix match except for the destinations above, so a job detail under
 * `/sync` still lights its parent and an organisation under `/admin/orgs`
 * lights that group's own entry rather than the overview beside it. */
export function isCurrent(pathname: string, href: string): boolean {
	if (EXACT_ONLY.includes(href)) {
		return pathname === href;
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
	{ from: '/jobs', to: '/sync' },
	{ from: '/listings', to: '/inventory' },
	{ from: '/connections', to: '/marketplaces' },
	{ from: '/settings/devices', to: '/marketplaces' },
	{ from: '/queue', to: '/reconciliation' },
	{ from: '/library', to: '/resources' }
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

/** Where the top-bar search sends the browser. A blank query is the inventory
 *  board with no filter rather than an empty `?q=`. */
export function searchHref(query: string): string {
	const trimmed = query.trim();
	return trimmed.length === 0 ? '/inventory' : `/inventory?q=${encodeURIComponent(trimmed)}`;
}
