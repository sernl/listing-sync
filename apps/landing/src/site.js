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
 * The address the founder actually monitors. Null until one exists, and null
 * renders no address at all rather than a `mailto:` that reaches nobody: the
 * footer drops the link and both legal pages say a contact address is still to
 * come. `hello@teachouse.io` stood here and was never monitored.
 */
export const supportEmail = null;

/**
 * The public download for the desktop client, which a seller needs before TPT
 * or TES work can run. Null renders as "Download link to come" rather than as
 * a broken link.
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
export const availability = 'TPT and TES connections work today.';
