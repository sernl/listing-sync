/**
 * Every value the founder must supply before this site goes live is here, and
 * nowhere else. The list of what still needs replacing is in
 * `docs/notes/design/landing-page.md` under "Placeholders".
 */

/** Where the console is served. Replace when the console's host is settled. */
export const consoleOrigin = 'https://app.teachouse.io';

export const signInUrl = `${consoleOrigin}/login`;

/** Replace with the address the founder actually monitors. */
export const supportEmail = 'hello@teachouse.io';

/**
 * Where a waitlist entry goes. Payments are not decided, so the site asks to
 * be told rather than asking to be paid. A `mailto:` stands in because it
 * needs no backend and keeps the site's property of making no third-party
 * request; replacing it with a posted form means changing `waitlistHref` and
 * nothing else on the site.
 */
export const waitlistEmail = 'hello@teachouse.io';

export const waitlistHref = `mailto:${waitlistEmail}?subject=${encodeURIComponent(
	'Teachouse waitlist'
)}`;

/**
 * The public download for the desktop client, which a seller needs before
 * TeachersPayTeachers or Tes work can run. Null renders as "Download link to
 * come" rather than as a broken link.
 *
 * There is no URL to put here yet. Releases go to CrabNebula Cloud on the
 * `beta` channel, and CrabNebula's documentation says a channelled release is
 * not listed on an application's public page; the GitHub releases beside them
 * are in a private repository. See `docs/notes/design/desktop-distribution.md`.
 */
export const downloadUrl = null;

/**
 * What the founder is willing to say about availability today. This sentence
 * is the only claim on the site about whether a seller can use it right now.
 */
export const availability =
	'Teachouse is in private testing. TeachersPayTeachers and Tes connections work today, and Etsy is next.';

/**
 * Transport class per marketplace, matching `InventoryId::transport_class` in
 * `crates/tam-domain/src/registry`. `device` means every request to that
 * marketplace originates on the seller's own machine under the seller's own
 * login; `api` means the marketplace publishes an official API and issues us a
 * token for the purpose.
 */
export const marketplaces = [
	{
		name: 'TeachersPayTeachers',
		transport: 'device',
		status: 'Working today'
	},
	{
		name: 'Tes',
		transport: 'device',
		status: 'Working today'
	},
	{
		name: 'Etsy',
		transport: 'api',
		status: 'Next'
	},
	{
		name: 'More marketplaces',
		transport: 'either',
		status: 'Planned'
	}
];

export const transportWording = {
	device: 'Built to run on your computer, under your own login',
	api: 'Runs on our servers, over the official API',
	either: 'Whichever of the two the marketplace sanctions'
};
