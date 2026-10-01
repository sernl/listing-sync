/**
 * Every value the founder must supply before this site goes live is here, and
 * nowhere else. The list of what still needs replacing is in
 * `docs/notes/design/landing-page.md` under "Placeholders".
 */

/**
 * The console is served from this same origin, so both buttons are a path
 * rather than a URL and the site keeps working under `default-src 'self'`.
 */
export const loginUrl = '/login';

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
