/**
 * Every value the founder must supply before this site goes live is here, and
 * nowhere else. The list of what still needs replacing is in
 * `docs/notes/design/landing-page.md` under "Placeholders".
 */

/**
 * Where the console lives. It has its own host, so every link into it from
 * this site is an absolute URL built from this; navigation is not something
 * `default-src 'self'` restricts, so the policy is unchanged. A build for
 * local development sets `PUBLIC_CONSOLE_URL` to the console it runs beside.
 * `import.meta.env` is read optionally because `just landing-copy-gate`
 * imports this module under plain Node, where it is undefined.
 */
export const consoleUrl = import.meta.env?.PUBLIC_CONSOLE_URL || 'https://dash.teachouse.io';

/** The console's sign-in, which the header's "Log in" and the maintenance
 *  page's team link both go to. */
export const loginUrl = `${consoleUrl}/login`;

/**
 * The address the founder monitors, and the one the privacy policy and the
 * terms name for questions and requests. The footer, both legal pages and the
 * maintenance page all link it.
 */
export const supportEmail = 'contact@teachouse.io';

/**
 * What the founder is willing to say about availability today. This sentence
 * is the only claim on the site about whether a seller can use it right now.
 */
export const availability = 'TPT and Tes connections work today, with more marketplaces coming.';

/**
 * The home page's `<title>`, and so the line a search result shows. It names
 * the two marketplaces that connect today because that is what a seller types
 * into a search box; like `availability`, it is the one other string the copy
 * gate lets name them, and it is read from here so the gate holds no copy.
 */
export const homeTitle = 'Teachouse \u2013 publish and sync your TPT and Tes listings';
