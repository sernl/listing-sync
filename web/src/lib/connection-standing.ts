// Which stored connection states mean the seller has a marketplace here, and
// which mean they do not.
//
// One module because three screens ask it and they must not disagree: the
// marketplaces page decides whether to offer a disconnect, the import page
// badges a card Connected or Not connected, and the migration page decides
// whether there is a shop to bring across. Before this, two of them answered
// on the row's presence alone, so a marketplace the seller had disconnected
// went on reading as connected everywhere but the card that stated its status.
//
// The distinction that matters is between "off" and "unwell". `needs_reauth`
// is a connection the seller still has and their own device is the thing that
// discovers the sign-in is needed, so it stands; the two states below are the
// seller or an operator having taken the connection away.

import type { ConnectionView } from '$lib/api';

/** The stored states that mean there is no connection to speak of.
 *
 * `unlinked` is what the seller's own Disconnect writes and `revoked` is the
 * operator and security path. Both read as `disconnected` to the seller
 * through `connection_status` in Rust, and neither is a marketplace this
 * console may treat as the seller's. */
export const CONNECTION_OFF: readonly string[] = ['unlinked', 'revoked'];

/**
 * Whether this connection is one the seller still has.
 *
 * Presence and state together, deliberately not health: a connection needing a
 * fresh sign-in is still the seller's shop and it is their device that
 * discovers the sign-in is needed rather than any of these screens. What this
 * excludes is only the two states that say the connection was given up.
 */
export function connectionStands(connection: { state: string } | null | undefined): boolean {
	return (
		connection !== null && connection !== undefined && !CONNECTION_OFF.includes(connection.state)
	);
}

/**
 * Whether the seller has any marketplace connection at all.
 *
 * The question a screen asks before it says "nothing is connected", and it has
 * to be asked over every connection rather than over the one that screen can
 * use. A migration reads TES and nothing else, so "no source for a migration"
 * and "no marketplace connected" are different facts, and the second one is
 * false for a seller who connected TPT first — which is the ordinary order,
 * since TPT is where a migration writes.
 */
export function anyConnectionStands(connections: readonly ConnectionView[]): boolean {
	return connections.some((connection) => connectionStands(connection));
}

/**
 * Whether a machine reported this marketplace connected at the last check-in.
 *
 * `linked` alone, and it is the field the disconnect route and `derive_link`
 * both operate on: `derive_link` writes it when any live device reports a
 * session and lifts an `unlinked` row back to it on the next beat. So it is
 * also the honest answer to "will a machine reconnect this by itself", which
 * is what the browser disconnect has to warn about.
 *
 * Deliberately not the device registry's own sign-in state. That is a second
 * computation of the same underlying heartbeat, and two paths answering one
 * question is exactly what this module exists to stop.
 */
export function connectionIsLinked(connection: { state: string } | null | undefined): boolean {
	return connection !== null && connection !== undefined && connection.state === 'linked';
}

/** The marketplaces the seller still has a connection for, as a set the
 *  callers already wanted. */
export function standingMarketplaces(
	connections: readonly ConnectionView[]
): Set<ConnectionView['marketplace']> {
	return new Set(
		connections.filter((connection) => connectionStands(connection)).map((one) => one.marketplace)
	);
}
