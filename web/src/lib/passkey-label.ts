// The words the settings list puts on a registered passkey. better-auth stores
// an optional label the browser supplied at registration and the credential's
// backup state; neither is a sentence a seller can read. Pure, so it tests
// without a component.

/** What the list renders off one row, structurally: better-auth's schema makes
 *  the label optional and the plugin returns the row verbatim. */
export interface LabelledPasskey {
	name?: string | null;
	backedUp?: boolean | null;
}

/** The fallback deliberately guesses at no device. better-auth records the
 *  label the browser chose to send and nothing else about the authenticator
 *  that a seller could check, so a passkey registered without one is named as
 *  unnamed rather than as a device it may not be. */
export function passkeyLabel(passkey: LabelledPasskey): string {
	return passkey.name?.trim() || 'Unnamed passkey';
}

/** What the credential's backup state means for the seller: whether losing the
 *  device that made this passkey also loses the passkey. */
export function passkeyReach(passkey: LabelledPasskey): string {
	return passkey.backedUp
		? 'Synced by your password manager, so it works on your other devices.'
		: 'Held on the single device that made it.';
}
