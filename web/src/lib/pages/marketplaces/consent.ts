/**
 * The seller-device consent: which marketplaces need it, whether it stands,
 * and the notice the seller reads before agreeing.
 *
 * A marketplace with no official API is connected through the seller's own
 * sign-in on their own device, and the founder's rule is that this happens
 * only after the seller has read what that means and pressed "I agree". The
 * record lives on the server, per organisation; this module is the console's
 * reading of it and the prose the dialog shows. Nothing here is stored
 * locally, deliberately: a device is the party whose word the record exists
 * to stop being the only evidence.
 */

import type { ConsentsView } from '$lib/api';
import type { Marketplace } from '$lib/generated/vocab';
import type { MarketplaceRow } from '$lib/devices-view';
import { TRANSPORT_OF } from '$lib/inventory';
import { CARD_NAME } from './view';

/** The version of the notice a grant must carry to stand. Held identically
 *  in `crates/tam-types/src/lib.rs`; a material change to the prose below is
 *  a new date on both sides, and every older grant stops standing. */
export const CONSENT_NOTICE_VERSION = '2026-09-14';

/** The marketplaces that need the seller's consent: the ones with no official
 *  API, derived from the same transport table the cards read. */
export const SELLER_DEVICE: readonly Marketplace[] = (
	Object.keys(TRANSPORT_OF) as Marketplace[]
).filter((marketplace) => TRANSPORT_OF[marketplace] === 'SellerDevice');

export function needsConsent(marketplace: Marketplace): boolean {
	return SELLER_DEVICE.includes(marketplace);
}

/** Where one marketplace's grant stands.
 *
 *  `unknown` while the read has not landed, so a card does not offer Connect
 *  on a fact we never received. `outdated` is a grant standing on another
 *  notice: the seller agreed once, to text that no longer says what the
 *  current one says, and is asked to read the current one. */
export type ConsentStanding = 'unknown' | 'granted' | 'missing' | 'outdated';

export function standingFor(
	view: ConsentsView | undefined,
	marketplace: Marketplace
): ConsentStanding {
	if (view === undefined) {
		return 'unknown';
	}
	const open = view.consents.filter(
		(row) => row.marketplace === marketplace && row.withdrawn_at === null
	);
	if (open.some((row) => row.standing)) {
		return 'granted';
	}
	return open.length > 0 ? 'outdated' : 'missing';
}

export interface ConsentCopy {
	title: string;
	intro: string;
	grants: string[];
	boundary: string;
	checkbox: string;
}

/** The notice, in the seller's words. Every sentence names the marketplace,
 *  because the seller is agreeing to one connection and not to a policy. */
export function consentCopy(marketplace: Marketplace): ConsentCopy {
	const name = CARD_NAME[marketplace];
	return {
		title: `Connect your ${name} store`,
		intro: `${name} has no official API, so this connection uses your own ${name} sign-in on this device rather than a token ${name} issues.`,
		grants: [
			`Store and use your ${name} sign-in session on this device.`,
			`Read your store's listings and resource details.`,
			`Carry out ${name} actions you request or switch on, such as publishing, updating and importing.`,
			`Keep catalogue details, listing copy and cover images on Teachouse's servers to coordinate your account.`
		],
		boundary: `Your ${name} session stays on this device. This permission does not let Teachouse upload your resource files to its servers; that is asked for separately, if ever.`,
		checkbox: `I confirm I own or am authorised to manage this ${name} store, and I authorise the access, on-device session storage and server-side catalogue records described above.`
	};
}

/** The first linked seller-device marketplace with no standing grant, for
 *  the banner that says work has stopped. Linked means some machine of the
 *  seller's reports a session for it; a marketplace nobody is signed in to
 *  is asked for consent at Connect instead. */
export function blockedBanner(
	rows: readonly MarketplaceRow[],
	view: ConsentsView | undefined
): { marketplace: Marketplace; title: string } | null {
	for (const row of rows) {
		if (!needsConsent(row.marketplace) || row.signIn.state !== 'signed_in') {
			continue;
		}
		const standing = standingFor(view, row.marketplace);
		if (standing === 'missing' || standing === 'outdated') {
			return {
				marketplace: row.marketplace,
				title: `${CARD_NAME[row.marketplace]} needs your permission to keep working`
			};
		}
	}
	return null;
}
