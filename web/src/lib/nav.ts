// The console shell's own model: the icon rail's sections, the page list each
// one opens, the paths the old information architecture used, and the two
// small renderings the account card and the search box need. Pure, so it
// tests without a component.
//
// A section model rather than a flat group list, because the rail and the
// secondary navigation card are two renderings of one structure: the rail
// shows the sections, the card shows the current section's pages. A flat list
// cannot say which glyph owns which pages.

import type { IconName } from '$lib/icons';

export interface NavItem {
	href: string;
	label: string;
	/** The Lucide glyph this destination is drawn with. A closed union rather
	 *  than a string, so a name that is not shipped stops the lane instead of
	 *  rendering an empty box. */
	icon: IconName;
	/** Marks a destination that exists as a route and not yet as a feature. */
	soon?: true;
	/** Paths this destination answers for besides its own `href`.
	 *
	 *  A route's place in the URL is not always its place in the navigation.
	 *  `/sync/requests/<id>` is the one case: it lives under `/sync` because a
	 *  sync request is what carries it, and it is opened from the Marketplace
	 *  Migration list, so it belongs there rather than under the path it sits
	 *  beneath. */
	owns?: readonly string[];
}

export type SectionId =
	| 'import'
	| 'crosslist'
	| 'automations'
	| 'marketplaces'
	| 'account'
	| 'admin';

export interface NavSection {
	id: SectionId;
	/** The rail glyph's accessible name, and the navigation card's title. */
	label: string;
	/** The one sentence the section says for itself, in the founder's words.
	 *
	 *  One string on the section rather than two copies in markup, because the
	 *  rail's tooltip and the navigation card's lede are the same sentence and
	 *  a second copy is how they come to disagree. */
	hint: string;
	icon: IconName;
	/** Where the rail glyph itself lands. */
	href: string;
	/** The section-primary button, directly beneath the card's title. */
	primary?: NavItem;
	/** The pages the card lists. Empty for a section that has none, which
	 *  renders with no card at all and a correspondingly wider content
	 *  region. */
	items: readonly NavItem[];
}

/** The four sections the founder named, then Account. In rail order.
 *
 * Import leads, because bringing a portfolio in is the first thing a teacher
 * does and the founder's own list opens with it.
 *
 * Marketplaces carries no page list on purpose: its connections, browser
 * extensions and downloads are three headings on one scrolling page, which is
 * how Vendoo's own equivalent reads and what the founder's list describes. */
export const SECTIONS: readonly NavSection[] = [
	{
		id: 'import',
		label: 'Import',
		hint: 'Bring your current portfolio to Teachouse from anywhere it is housed.',
		icon: 'download',
		href: '/import',
		items: [
			{
				href: '/import',
				label: 'Import',
				icon: 'download',
				// A batch's own report sits at `/imports/<batch>`, one letter away
				// from this section's path and not underneath it, so the section has
				// to claim it by name or the breadcrumb answers "Console" on every
				// import a teacher opens.
				owns: ['/imports']
			}
		]
	},
	{
		id: 'crosslist',
		label: 'Crosslist',
		hint: 'Publish your resources to multiple marketplaces.',
		icon: 'package',
		href: '/resources',
		primary: { href: '/resources/new', label: 'New resource', icon: 'circle-plus' },
		items: [
			{ href: '/resources', label: 'Resources', icon: 'layout-list' },
			{ href: '/labels', label: 'Labels', icon: 'tag' },
			// Between Labels and Analytics because a collection is the other
			// selection dimension a seller files by: a label is a word on a
			// resource, a collection is an ordered set of them. `/collections/<id>`
			// needs no `owns` entry -- it falls under this href by prefix.
			{ href: '/collections', label: 'Collections', icon: 'layers' },
			{ href: '/analytics', label: 'Analytics', icon: 'chart-line' },
			{ href: '/templates', label: 'Template Manager', icon: 'layout-template' },
			{ href: '/export', label: 'Export', icon: 'file-down' }
		]
	},
	{
		id: 'automations',
		label: 'Automations',
		hint: 'Edit tags, descriptions, titles and files across your listings.',
		icon: 'waves-horizontal',
		// A landing page of its own, unlike the other sections, whose rail glyph
		// lands on their first page. `/automations` lists the three as cards, so
		// it is a destination `SECTION_LANDINGS` names rather than one of the
		// items below.
		href: '/automations',
		items: [
			// The path is unchanged although the word is: `/automations/sharing`
			// is where the page has always been, and moving it would break every
			// link a seller has kept.
			{ href: '/automations/sharing', label: 'Scheduling', icon: 'calendar-clock' },
			{
				href: '/automations/migration',
				label: 'Migrations',
				icon: 'arrow-right-left',
				// A past migrate request is opened from this page's own list, so
				// `/sync/requests/<id>` belongs here rather than under the path it
				// happens to sit beneath (D8).
				owns: ['/sync/requests']
			},
			// The sync list stays at `/sync` rather than moving under
			// `/automations/`, because `/sync/<id>` and `/sync/requests/<id>` are
			// its detail pages and a list that left its own children behind would
			// break both the breadcrumb and the lit nav entry.
			{ href: '/sync', label: 'Marketplace Sync', icon: 'refresh-cw' }
		]
	},
	{
		id: 'marketplaces',
		label: 'Marketplaces',
		hint: 'Connect the places you sell.',
		icon: 'store',
		href: '/marketplaces',
		items: []
	},
	{
		id: 'account',
		label: 'Account',
		hint: 'Your settings, plan and notifications.',
		icon: 'circle-user',
		href: '/settings',
		items: [
			{ href: '/settings', label: 'Preferences', icon: 'sliders-horizontal' },
			{ href: '/settings/subscription', label: 'Subscription', icon: 'credit-card' },
			{ href: '/notifications', label: 'Notifications', icon: 'bell' },
			{ href: '/status', label: 'Status', icon: 'activity' },
			{ href: '/guides', label: 'Help and guides', icon: 'book-open', soon: true }
		]
	}
];

/** The two destinations no section lists.
 *
 * `/app` is the console's home, which the desktop app and the public site both
 * navigate to; it answers with the Resources list rather than a dashboard of
 * its own, so it is named for what it shows. It keeps its own entry rather than
 * being folded into the Crosslist page list because the path is real and the
 * breadcrumb has to resolve it, and it lands on the same screen `/resources`
 * does. The reconciliation queue is reached from the Marketplace Sync screen,
 * which carries its count; the rail carries no badge of its own, because two
 * places showing one figure is one place too many. */
export const HOME_ITEM: NavItem = {
	href: '/app',
	label: 'Resources',
	icon: 'layout-list'
};

export const OPEN_QUESTIONS_ITEM: NavItem = {
	href: '/reconciliation',
	label: 'Open questions',
	icon: 'circle-question-mark'
};

const PINNED: readonly NavItem[] = [HOME_ITEM, OPEN_QUESTIONS_ITEM];

/** The operator's own group, rendered under the others and only for a human
 *  the operator probe admitted.
 *
 *  Separate from `NAV_GROUPS` rather than a flag inside it, because every
 *  entry there is unconditional: a seller sees exactly that sidebar, and a
 *  conditional item mixed into the same list is one `if` away from leaking a
 *  destination that answers 401. Its destinations still refuse a
 *  non-operator on their own — the hiding is courtesy, never the fence. */
export const ADMIN_SECTION: NavSection = {
	id: 'admin',
	label: 'Admin',
	hint: 'The operator views of this deployment.',
	icon: 'shield-check',
	href: '/admin',
	items: [
		{ href: '/admin', label: 'Overview', icon: 'layout-dashboard' },
		{ href: '/admin/orgs', label: 'Organisations', icon: 'building-2' },
		{ href: '/admin/health', label: 'Sync health', icon: 'heart-pulse' },
		{ href: '/admin/failures', label: 'Failed writes', icon: 'circle-x' },
		{ href: '/admin/import-drain', label: 'Import drain', icon: 'chart-line' },
		{ href: '/admin/users', label: 'Identity users', icon: 'users' },
		{ href: '/admin/impersonations', label: 'Impersonations', icon: 'copy' }
	]
};

/** A section whose landing path no page of its own covers is still a
 *  destination — Marketplaces is the whole of it — so it is named here rather
 *  than being reachable and nameless. Without this the breadcrumb answers
 *  "Console" on that page and a redirect aimed at it looks like a redirect to
 *  nothing. */
const SECTION_LANDINGS: readonly NavItem[] = [...SECTIONS, ADMIN_SECTION]
	.filter((section) => !section.items.some((item) => item.href === section.href))
	.map((section) => ({ href: section.href, label: section.label, icon: section.icon }));

/** Every destination this model names, however it is reached. The one list the
 *  breadcrumb, the redirect table's own test and the tab bar all read, so a
 *  destination cannot be reachable in the shell and absent from the model. */
export const ALL_DESTINATIONS: readonly NavItem[] = [
	...SECTIONS.flatMap((section) => section.items),
	...ADMIN_SECTION.items,
	...SECTION_LANDINGS,
	...PINNED
];

const ALL_ITEMS = ALL_DESTINATIONS;

/** Every path prefix a destination answers for: its own href, then anything it
 *  owns. */
function claims(item: NavItem): readonly string[] {
	return item.owns === undefined ? [item.href] : [item.href, ...item.owns];
}

/** The length of the longest claim this path falls under, or null where the
 *  destination does not answer for it at all.
 *
 * Length rather than a boolean, because resolution is longest-match: it is what
 * keeps `/sync/<id>` on Marketplace Sync while `/sync/requests/<id>` goes to
 * Import, and `/admin/orgs/<id>` on Organisations rather than Overview. */
function claimed(pathname: string, item: NavItem): number | null {
	let best: number | null = null;
	for (const claim of claims(item)) {
		if (isCurrent(pathname, claim) && (best === null || claim.length > best)) {
			best = claim.length;
		}
	}
	return best;
}

/** The section the rail lights, and whose pages the card lists, or `null` where
 *  the path belongs to no section.
 *
 * The longest matching page wins before the section's own landing path is
 * considered, so `/settings/subscription` opens Account rather than whichever
 * section happens to list a shorter prefix.
 *
 * `null` rather than a fallback, because the home path and the open-questions
 * queue belong to no section and a fallback lit Crosslist on both: the rail
 * claimed a section the seller was not in, and the card offered six pages none
 * of which was the one on screen. A path that belongs to no section has to be
 * sayable, and only a nullable answer says it. */
export function sectionFor(pathname: string, operator = false): NavSection | null {
	const sections = operator ? [...SECTIONS, ADMIN_SECTION] : SECTIONS;
	let best: { section: NavSection; length: number } | null = null;
	for (const section of sections) {
		for (const item of section.items) {
			const length = claimed(pathname, item);
			if (length !== null && (best === null || length > best.length)) {
				best = { section, length };
			}
		}
	}
	if (best !== null) {
		return best.section;
	}
	// No page matched, so the landing paths decide: a section with no page list
	// of its own is reachable only this way.
	return sections.find((section) => isCurrent(pathname, section.href)) ?? null;
}

/** Destinations matched exactly rather than by prefix, because each one has
 *  sibling destinations of its own beneath it in the same model: every
 *  operator page is under `/admin`, and Subscription is under `/settings`. A
 *  prefix match on either would light two entries at once. */
const EXACT_ONLY: readonly string[] = ['/admin', '/settings'];

/** The rail's sections rendered as tabs. `PHONE_BAR` below is the bar the
 *  console actually shows; this is the navigating half of it, so the tabs and
 *  the rail cannot come to name different destinations.
 *
 *  This is the Android app's navigation too, because that build shows this
 *  same console (`apps/desktop/src-tauri/tauri.conf.json`).
 *
 *  Derived from `SECTIONS`, so a renamed section renames its tab or stops the
 *  lane at module load rather than rotting silently. */
export const SECTION_TABS: readonly NavItem[] = SECTIONS.map((section) => ({
	href: section.href,
	label: section.label,
	icon: section.icon
}));

export interface PhoneTab extends Omit<NavItem, 'href'> {
	/** Where the cell goes. Absent on a cell that is an action rather than a
	 *  destination, so it has no path to carry and no `aria-current` to take. */
	href?: string;
	/** The word the bar draws, where the section's own name is too long for a
	 *  cell. The rail keeps `label`: a 72px cell at 12px holds about nine
	 *  characters and "Marketplaces" is twelve, so the bar needs its own
	 *  wording rather than the sections needing renaming. `label` stays the
	 *  accessible name, so the two never disagree about where a cell goes. */
	short?: string;
	/** The create action. Drawn as a filled disc rather than as a tab,
	 *  because it is the one thing on the bar that is not a place to go. */
	create?: true;
}

/** The create action the phone bar carries, taken from the Crosslist section's
 *  own primary rather than written again, so the phone button and the two
 *  desktop ones cannot come to open different screens. */
const createAction = SECTIONS.find((section) => section.id === 'crosslist')?.primary;
if (createAction === undefined) {
	throw new Error('the phone bar carries the Crosslist create action, which that section has none of');
}

/** `href` narrowed back to a definite string: the top strip's own create link
 *  reads this href directly. */
export const CREATE_TAB: PhoneTab & { href: string } = {
	href: createAction.href,
	// The section-primary's own words, so the three create controls — the
	// navigation card, the top strip and this — cannot come to read
	// differently. The bar draws `short` and announces `label`, because the
	// centre cell carries a 48px disc above its word and has no room for two
	// of them.
	label: createAction.label,
	short: 'New',
	// A bare plus rather than the card's `circle-plus`: the button is already a
	// filled circle, and a ring inside a disc reads as a mistake.
	icon: 'plus',
	create: true
};

const accountSection = SECTIONS.find((section) => section.id === 'account');
if (accountSection === undefined) {
	throw new Error('the shell reaches Account from the page header, which the rail has none of');
}

/** Account, as the one destination the page header's avatar button opens.
 *
 * Taken from the section rather than written again, so a renamed section
 * renames the button. It is not a phone-bar cell: the founder's 2026-09-12
 * review of the seven-cell bar -- too many icons, and the new-resource action
 * has to sit in the middle -- took Account off it, and a top-right avatar on
 * every page header is where a phone application puts secondary
 * administration. `PageHead.svelte` draws it below 620px only; above that the
 * rail and the top strip already carry it. */
export const ACCOUNT_DESTINATION: NavItem = {
	href: accountSection.href,
	label: accountSection.label,
	icon: accountSection.icon
};

/** What a section is called and drawn as on the phone bar.
 *
 * Three of the four navigating sections need bar-only wording and two need a
 * bar-only glyph: `package` and `waves-horizontal` are the rail's marks for
 * Crosslist and Automations, and at 24px over a nine-character word they read
 * as a box and a river rather than as a catalogue and a workflow. Keyed by
 * section id, so a section that gains a cell is a line here rather than a
 * second list of destinations. */
const BAR_CELLS: Partial<Record<SectionId, { short: string; icon: IconName }>> = {
	crosslist: { short: 'Catalogue', icon: 'library-big' },
	automations: { short: 'Automate', icon: 'workflow' },
	marketplaces: { short: 'Markets', icon: 'store' }
};

/** The bar's navigating cells: every section but Account, in rail order.
 *
 * Derived from `SECTIONS` rather than listed again, so a section added to the
 * rail reaches the bar instead of being dropped by a slice no test would
 * notice. */
const NAVIGATING: readonly PhoneTab[] = SECTIONS.filter((section) => section.id !== 'account').map(
	(section) => {
		const cell = BAR_CELLS[section.id];
		return {
			href: section.href,
			label: section.label,
			icon: cell?.icon ?? section.icon,
			...(cell === undefined ? {} : { short: cell.short })
		};
	}
);

/** The middle of the bar, computed rather than written as 2: an even number of
 *  navigating cells is exactly what lets the create action sit on the bar's own
 *  centre line, so a fifth navigating section has to break loudly here instead
 *  of quietly moving the disc off centre on a phone. */
const CENTRE = NAVIGATING.length / 2;
if (!Number.isInteger(CENTRE)) {
	throw new Error(
		`the phone bar centres the create action, which ${NAVIGATING.length} navigating cells cannot do`
	);
}

/** The phone bar: four navigating sections with the create action in the exact
 * middle of them.
 *
 * Five cells on five equal tracks, which is the whole of what the founder
 * asked for on 2026-09-12. Search and Account left the bar to make room:
 * `ACCOUNT_DESTINATION` above records where Account went, and the Resources
 * page header carries the search control that opens the same palette Ctrl-K
 * does. */
export const PHONE_BAR: readonly PhoneTab[] = [
	...NAVIGATING.slice(0, CENTRE),
	CREATE_TAB,
	...NAVIGATING.slice(CENTRE)
];

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

/** The page's own name, which the shell sets the document title from. The
 *  longest matching destination wins, so `/sync/<id>` reads as Marketplace
 *  Sync rather than as the first nav entry that happens to prefix it. */
export function breadcrumbFor(pathname: string): string {
	return currentDestination(pathname)?.label ?? 'Console';
}

/** The one destination this path belongs to, by longest claim, or null.
 *
 * Longest rather than first, and across claims rather than hrefs, because both
 * matter: `/sync/<id>` belongs to Marketplace Sync while `/sync/requests/<id>`
 * belongs to Import, which owns that prefix without containing it. A caller
 * asking `isCurrent` per item instead gets two answers on the first path and
 * none on the second. */
export function currentDestination(pathname: string): NavItem | null {
	let best: { item: NavItem; length: number } | null = null;
	for (const item of ALL_ITEMS) {
		const length = claimed(pathname, item);
		if (length !== null && (best === null || length > best.length)) {
			best = { item, length };
		}
	}
	return best?.item ?? null;
}

/** Where a path from the old information architecture now lives.
 *
 * Prefixes rather than whole paths: `/jobs/<id>` has to land on the same job
 * under its new name, and a redirect table listing identifiers could not. */
export const LEGACY_REDIRECTS: readonly { from: string; to: string }[] = [
	{ from: '/jobs', to: '/sync' },
	// One hop, not two: `/listings` was pointed at the catalogue when the
	// catalogue was `/inventory`, and a table that walked through the old name
	// to reach the new one would redirect twice for every saved link.
	{ from: '/listings', to: '/resources' },
	{ from: '/connections', to: '/marketplaces' },
	{ from: '/settings/devices', to: '/marketplaces' },
	{ from: '/queue', to: '/reconciliation' },
	// The help placeholder was `/library`, then `/resources`; it is `/guides`
	// now, because "Resources" is what the catalogue is called and two
	// destinations cannot share a name.
	{ from: '/library', to: '/guides' },
	// The catalogue answered at `/inventory` until the seller-facing noun won:
	// the rail says Resources, so the path does. Permanent, because a bookmark
	// on one resource and every link already sent out name the old path.
	{ from: '/inventory', to: '/resources' },
	// `/help` and `/guides` were two placeholders for one thing, and nothing
	// pointed at `/help` any more.
	{ from: '/help', to: '/guides' }
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

/** The pages a browser with no API session may be shown. `/status` is
 *  deliberately among them: it matters most when signing in is what is broken.
 *
 * Route knowledge, so it lives with the rest of it rather than as an array
 * inside the layout that reads it. That is not tidying: the layout renders the
 * signed-out branch on the session alone, and a console page's markup therefore
 * mounts and fires its queries for a frame before the redirect lands. The
 * branch could not ask whether the route was public because the answer was a
 * local `const` — and being local, nothing tested it either.
 *
 * Whole paths rather than prefixes, which is why `/reset/confirm` is named
 * beside `/reset`: a prefix here would make every path under a public one
 * public too, and `/status` is one segment away from paths that are not.
 */
export const PUBLIC_ROUTES: readonly string[] = [
	'/login',
	'/signup',
	'/reset',
	'/reset/confirm',
	'/status'
];

/** What a signed-out browser is shown at this path.
 *
 * `redirecting` is a screen rather than the absence of one. The layout's effect
 * navigates to `/login`, and a render is what stands in the meantime: without
 * it the console's own page mounts behind the navigation, and with a bare
 * fallback it is the browser's error template that appears instead. */
export type SignedOutView = 'public' | 'redirecting';

export function signedOutView(pathname: string): SignedOutView {
	return PUBLIC_ROUTES.includes(pathname) ? 'public' : 'redirecting';
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

/** What the account tile draws, decided once for the top strip and the phone
 *  bar so the two cannot disagree: the seller's own picture where one is set,
 *  the organisation's initials otherwise, and the section glyph while neither
 *  is known -- every frame before the reads land, and a name with no letter. */
export type AccountTile =
	| { kind: 'picture'; src: string }
	| { kind: 'initials'; text: string }
	| { kind: 'glyph' };

export function accountTile(
	pictureSrc: string | null,
	orgName: string | undefined | null
): AccountTile {
	if (pictureSrc !== null) {
		return { kind: 'picture', src: pictureSrc };
	}
	const text = initialsOf(orgName);
	return text === '' ? { kind: 'glyph' } : { kind: 'initials', text };
}

/** Where a query with more matches than the palette shows sends the browser:
 *  the catalogue board, filtered to that same query. The top bar no longer
 *  holds a search box -- it opens the Ctrl-K palette -- so the palette's
 *  overflow link is the one caller. A blank query is the board with no filter
 *  rather than an empty `?q=`. */
export function searchHref(query: string): string {
	const trimmed = query.trim();
	return trimmed.length === 0 ? '/resources' : `/resources?q=${encodeURIComponent(trimmed)}`;
}
