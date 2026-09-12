/** Whether the command palette is open.
 *
 * A module store because the palette's element lives in `Console.svelte` --
 * it floats over whichever page is drawn, so it cannot be owned by the page --
 * while two of the controls that open it do not: the Resources page header
 * carries the phone's search button, which is where search went when it left
 * the phone bar on 2026-09-12. Passing a callback down through the page
 * components to reach it would thread an opener through every page that never
 * opens one.
 *
 * Global rather than context-scoped: one console renders one palette, and the
 * Ctrl-K handler in the shell writes this same flag. */

let open = $state(false);

export const palette = {
	get open() {
		return open;
	},

	set open(next: boolean) {
		open = next;
	},

	/** What every opening control calls, so none of them has to know the flag's
	 *  name or whether opening will one day mean more than setting it. */
	show() {
		open = true;
	}
};
