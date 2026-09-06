// The organisation-slug rule as the claim screen and the settings form apply
// it, mirroring `validated_slug` in `crates/tam-api/src/org.rs`. Pure, so it
// tests without a component.
//
// The client check spares a round trip for a slug the server would refuse, and
// it spares the availability endpoint a probe on a slug that cannot be claimed
// whatever the answer. It does not stand in for the server's check: the form
// renders the 422 body whenever the two disagree, and it renders the 409 even
// where this module and the availability probe both said the slug was free,
// because check-then-write is a race only the database's unique index settles.
//
// The verdict carries the normalised slug rather than the raw one, so the
// screen can show the seller the lowercased form they would actually get.

/** The server's bounds. ASCII by construction -- the shape rule admits nothing
 *  else -- so counting code units and counting characters agree here, unlike
 *  the display name in `org-name.ts`. */
export const SLUG_MIN_CHARS = 3;
/** See {@link SLUG_MIN_CHARS}. */
export const SLUG_MAX_CHARS = 32;

/** The handles no seller may claim, because this product already means
 *  something by them. Mirrors `RESERVED_SLUGS` in `crates/tam-api/src/org.rs`,
 *  which is the authority; `org-slug.test.ts` fails if the two drift, and also
 *  fails if a top-level path this product serves is absent from either. */
export const RESERVED_SLUGS: readonly string[] = [
	'abuse',
	'account',
	'admin',
	'analytics',
	'api',
	'app',
	'assets',
	'auth',
	'automations',
	'billing',
	'blog',
	'brand',
	'cdn',
	'connections',
	'docs',
	'downloads',
	'email',
	'export',
	'fonts',
	'guides',
	'help',
	'import',
	'imports',
	'inventory',
	'jobs',
	'labels',
	'library',
	'listings',
	'login',
	'mail',
	'marketplaces',
	'notifications',
	'postmaster',
	'pricing',
	'privacy',
	'purchases',
	'queue',
	'reconciliation',
	'reset',
	'resources',
	'root',
	'security',
	'settings',
	'signup',
	'static',
	'status',
	'support',
	'sync',
	'system',
	'teachouse',
	'templates',
	'terms',
	'vendors',
	'webmaster',
	'www',
];

/** Why a slug was refused, in the classes the server distinguishes. */
export type SlugProblem = 'empty' | 'characters' | 'hyphens' | 'too-short' | 'too-long' | 'reserved';

export type SlugVerdict =
	| { accepted: true; slug: string }
	| { accepted: false; problem: SlugProblem; message: string };

const SHAPE = /^[a-z0-9-]*$/;

export function checkOrgSlug(raw: string): SlugVerdict {
	const slug = raw.trim().toLowerCase();
	if (slug.length === 0) {
		return {
			accepted: false,
			problem: 'empty',
			message: 'Choose a name for your organisation.'
		};
	}
	if (!SHAPE.test(slug)) {
		return {
			accepted: false,
			problem: 'characters',
			message: 'Use letters, numbers and hyphens only.'
		};
	}
	if (slug.startsWith('-') || slug.endsWith('-') || slug.includes('--')) {
		return {
			accepted: false,
			problem: 'hyphens',
			message: 'Hyphens go between words, so not at the start or end, and never two in a row.'
		};
	}
	if (slug.length < SLUG_MIN_CHARS) {
		return {
			accepted: false,
			problem: 'too-short',
			message: `A name is at least ${SLUG_MIN_CHARS} characters.`
		};
	}
	if (slug.length > SLUG_MAX_CHARS) {
		return {
			accepted: false,
			problem: 'too-long',
			message: `A name is at most ${SLUG_MAX_CHARS} characters.`
		};
	}
	// A 32-character hexadecimal string is a UUID with its hyphens removed.
	// The server refuses one so the slug namespace can never collide with the
	// identifier namespace; the rule reaches every 32-character slug written
	// in `0-9a-f`, which is the rule meaning what it says rather than an
	// oversight.
	const unhyphenatedUuid = slug.length === SLUG_MAX_CHARS && /^[0-9a-f]+$/.test(slug);
	if (unhyphenatedUuid || RESERVED_SLUGS.includes(slug)) {
		return {
			accepted: false,
			problem: 'reserved',
			message: 'That name is reserved. Try another.'
		};
	}
	return { accepted: true, slug };
}

/** A display name proposed from a slug, for the claim screen's optional second
 *  field. Each hyphen-separated word takes an initial capital, so `riverbend-resources`
 *  proposes `Riverbend Resources`.
 *
 *  A proposal rather than a derivation: the seller edits it, and once they have
 *  the screen stops re-proposing, so a name they typed is never overwritten by
 *  a later keystroke in the slug field. */
export function nameFromSlug(slug: string): string {
	return slug
		.split('-')
		.filter((word) => word.length > 0)
		.map((word) => word.charAt(0).toUpperCase() + word.slice(1))
		.join(' ');
}
