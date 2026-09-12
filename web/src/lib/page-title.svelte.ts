/** The document title, where the page knows a better one than the shell does.
 *
 * `Console.svelte` sets the title from the breadcrumb, which is the nav
 * destination's own label and is right for every page whose name is fixed. A
 * guide's is not: `/guides/<slug>` is one destination with as many names as
 * there are guides, and "Help and guides · Teachouse" on all of them makes a
 * browser's history and a pinned tab useless.
 *
 * A module store rather than a second `<svelte:head><title>` in the page,
 * because two titles in one document are not an override: the browser takes
 * the first element it finds, which is the shell's. One writer at a time, and
 * the writer clears it on the way out — a stale override would rename the next
 * page.
 */

let override = $state<string | null>(null);

export const pageTitle = {
	/** What the shell should show, or null to use the breadcrumb. */
	get override(): string | null {
		return override;
	},

	/** Claim the title. Called from an effect whose teardown releases it, so a
	 *  page that leaves before its read lands never holds the title. */
	claim(name: string) {
		override = name;
	},

	release() {
		override = null;
	}
};
