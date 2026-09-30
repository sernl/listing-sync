/** Whether the bell may read the inbox, for the two places it is drawn: the
 *  top strip above the phone breakpoint, and the page header's phone tools
 *  below it.
 *
 * The same shape as `account-tile.svelte.ts`, for the same reason:
 * `Console.svelte` is the one component that renders only under a session, so
 * it is the only writer. A `PageHead` on a public page -- `/status` signed out
 * -- draws no bell and issues no read of its own. */

let live = $state(false);

export const bellHost = {
	get live(): boolean {
		return live;
	},

	/** Called by the console as it mounts, and with `false` as it goes. */
	set(next: boolean) {
		live = next;
	}
};

/** How many rows the bell lists: the newest twenty. */
export const BELL_SIZE = 20;
