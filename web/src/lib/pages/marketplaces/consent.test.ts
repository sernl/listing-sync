import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import type { ConsentsView, ConsentView } from '$lib/api';
import type { MarketplaceRow, SignInState } from '$lib/devices-view';
import type { Marketplace } from '$lib/generated/vocab';
import {
	CONSENT_NOTICE_VERSION,
	SELLER_DEVICE,
	blockedBanner,
	consentCopy,
	needsConsent,
	standingFor
} from './consent';
import { CARD_NAME } from './view';

function grant(marketplace: Marketplace, over: Partial<ConsentView> = {}): ConsentView {
	return {
		marketplace,
		notice_version: CONSENT_NOTICE_VERSION,
		granted_at: 1_000,
		withdrawn_at: null,
		standing: true,
		...over
	};
}

function view(...consents: ConsentView[]): ConsentsView {
	return { notice_version: CONSENT_NOTICE_VERSION, consents };
}

function row(marketplace: Marketplace, state: SignInState): MarketplaceRow {
	return {
		marketplace,
		transport: marketplace === 'Etsy' ? 'OfficialApi' : 'SellerDevice',
		signIn: { marketplace, state, device: null, accountLabel: null, line: 'x', tone: 'ok' },
		connection: null,
		quiet: false,
		wipeOutstandingOn: []
	};
}

describe('which marketplaces need consent', () => {
	it('is the seller-device branch and nothing else', () => {
		expect(SELLER_DEVICE).toEqual(expect.arrayContaining(['Tes', 'Tpt']));
		expect(needsConsent('Tes')).toBe(true);
		expect(needsConsent('Tpt')).toBe(true);
		expect(needsConsent('Etsy')).toBe(false);
	});

	// The version is held on both sides of the wire; a change to one that
	// misses the other makes every grant stop standing or never stand.
	it('names the notice version the server holds', () => {
		const types = readFileSync(
			fileURLToPath(new URL('../../../../../crates/tam-types/src/lib.rs', import.meta.url)),
			'utf8'
		);
		expect(types).toContain(`CONSENT_NOTICE_VERSION: &str = "${CONSENT_NOTICE_VERSION}"`);
	});
});

describe('the notice', () => {
	it('names the marketplace in every part and lists the four grants', () => {
		for (const marketplace of SELLER_DEVICE) {
			const copy = consentCopy(marketplace);
			const name = CARD_NAME[marketplace];
			expect(copy.title).toContain(name);
			expect(copy.intro).toContain(name);
			expect(copy.boundary).toContain(name);
			expect(copy.checkbox).toContain(name);
			expect(copy.grants).toHaveLength(4);
			expect(copy.grants[0]).toContain('sign-in session on this device');
			expect(copy.grants[3]).toContain("Teachouse's servers");
			expect(copy.boundary).toContain('does not let Teachouse upload your resource files');
		}
	});
});

describe('where a grant stands', () => {
	it('is unknown until the read lands', () => {
		expect(standingFor(undefined, 'Tpt')).toBe('unknown');
	});

	it('is granted on a standing row, missing with none, and outdated on another version', () => {
		expect(standingFor(view(grant('Tpt')), 'Tpt')).toBe('granted');
		expect(standingFor(view(grant('Tpt')), 'Tes')).toBe('missing');
		expect(standingFor(view(), 'Tpt')).toBe('missing');
		expect(
			standingFor(
				view(grant('Tpt', { withdrawn_at: 2_000, standing: false })),
				'Tpt'
			)
		).toBe('missing');
		expect(
			standingFor(
				view(grant('Tpt', { notice_version: '2020-01-01', standing: false })),
				'Tpt'
			)
		).toBe('outdated');
	});
});

describe('the banner that says work has stopped', () => {
	it('names the first linked seller-device marketplace with no standing grant', () => {
		const rows = [row('Tpt', 'signed_in'), row('Tes', 'signed_in')];
		expect(blockedBanner(rows, view())).toEqual({
			marketplace: 'Tpt',
			title: 'TPT needs your permission before work continues'
		});
		expect(blockedBanner(rows, view(grant('Tpt')))?.marketplace).toBe('Tes');
		expect(blockedBanner(rows, view(grant('Tpt'), grant('Tes')))).toBeNull();
	});

	it('says nothing before the read lands, for an unlinked marketplace, or for Etsy', () => {
		expect(blockedBanner([row('Tpt', 'signed_in')], undefined)).toBeNull();
		expect(blockedBanner([row('Tpt', 'needs_signin')], view())).toBeNull();
		expect(blockedBanner([row('Etsy', 'signed_in')], view())).toBeNull();
	});
});
