// The one way out of the console.
//
// Ending a session here is two calls rather than one -- the identity service's
// and the API's -- and what follows them counts as much: a cache left standing
// holds the previous seller's reads, and a route left standing holds their
// organisation. Two controls sign out, and they must not do it differently.
// The shell's account nav-card is one; the Preferences page is the other, and
// it is the only one of the two a phone can reach, because `shell.css` hides
// the nav-card below 620px.

import { goto, invalidateAll } from '$app/navigation';
import type { QueryClient } from '@tanstack/svelte-query';
import { signOutEverywhere } from '$lib/auth-client';
import { toast } from '$lib/toast';
import { setLedgerScope } from '$lib/ledger';

/** End both sessions, drop everything read under them, and return to sign-in.
 *
 * The client is passed in rather than read from context, because the layout
 * that creates it cannot read the context it provides. */
export async function signOut(queryClient: QueryClient): Promise<void> {
	setLedgerScope(null);
	const complete = await signOutEverywhere();
	if (!complete) {
		toast('error', 'Signed out here, but one of the two sessions may still be open.');
	}
	queryClient.clear();
	await invalidateAll();
	await goto('/login');
}
