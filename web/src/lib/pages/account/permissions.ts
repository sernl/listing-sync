/**
 * The Account page's "Marketplace permissions" panel: one row per marketplace
 * that needs the seller's consent, and the record beneath it.
 */

import type { ConsentsView, ConsentView } from '$lib/api';
import type { Marketplace } from '$lib/generated/vocab';
import { CARD_NAME } from '$lib/pages/marketplaces/view';
import {
	type ConsentStanding,
	SELLER_DEVICE,
	standingFor
} from '$lib/pages/marketplaces/consent';

export interface PermissionRow {
	marketplace: Marketplace;
	name: string;
	standing: ConsentStanding;
	/** The pill's word and tone. */
	pill: { label: string; tone: 'ok' | 'warn' | 'soon' };
	/** Which one button the row offers. */
	action: 'grant' | 'withdraw' | null;
}

function dateOf(millis: number): string {
	return new Date(millis).toLocaleDateString();
}

/** One row per seller-device marketplace, whatever the read did: an unread
 *  row says it is being checked rather than guessing. */
export function permissionRows(view: ConsentsView | undefined): PermissionRow[] {
	return SELLER_DEVICE.map((marketplace) => {
		const name = CARD_NAME[marketplace];
		const standing = standingFor(view, marketplace);
		const granted = view?.consents.find(
			(row) => row.marketplace === marketplace && row.withdrawn_at === null && row.standing
		);
		switch (standing) {
			case 'unknown':
				return { marketplace, name, standing, pill: { label: 'Checking…', tone: 'soon' }, action: null };
			case 'granted':
				return {
					marketplace,
					name,
					standing,
					pill: { label: `Granted ${dateOf(granted?.granted_at ?? 0)}`, tone: 'ok' },
					action: 'withdraw'
				};
			case 'outdated':
				return {
					marketplace,
					name,
					standing,
					pill: { label: 'Confirm again', tone: 'warn' },
					action: 'grant'
				};
			case 'missing':
				return { marketplace, name, standing, pill: { label: 'Not granted', tone: 'warn' }, action: 'grant' };
		}
	});
}

/** The `window.confirm` body before a withdrawal. */
export function withdrawPrompt(name: string): string {
	return (
		`Withdraw permission for ${name}?\n\n` +
		`Teachouse stops all new ${name} work on your machines. ` +
		`Your ${name} logins stay on each machine until you sign out there.`
	);
}

export interface HistoryLine {
	key: string;
	text: string;
}

/** The record, newest first, one line per grant and one per withdrawal. */
export function historyLines(consents: readonly ConsentView[]): HistoryLine[] {
	const lines: { at: number; key: string; text: string }[] = [];
	for (const row of consents) {
		const name = CARD_NAME[row.marketplace];
		lines.push({
			at: row.granted_at,
			key: `${row.marketplace}-${row.granted_at}-granted`,
			text: `${name} permission granted ${dateOf(row.granted_at)} (notice ${row.notice_version}).`
		});
		if (row.withdrawn_at !== null) {
			lines.push({
				at: row.withdrawn_at,
				key: `${row.marketplace}-${row.granted_at}-withdrawn`,
				text: `${name} permission withdrawn ${dateOf(row.withdrawn_at)}.`
			});
		}
	}
	return lines
		.sort((left, right) => right.at - left.at)
		.map(({ key, text }) => ({ key, text }));
}
