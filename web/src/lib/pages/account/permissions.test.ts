import { describe, expect, it } from 'vitest';
import type { ConsentsView, ConsentView } from '$lib/api';
import type { Marketplace } from '$lib/generated/vocab';
import { CONSENT_NOTICE_VERSION, SELLER_DEVICE } from '$lib/pages/marketplaces/consent';
import { historyLines, permissionRows, withdrawPrompt } from './permissions';

function grant(marketplace: Marketplace, over: Partial<ConsentView> = {}): ConsentView {
	return {
		marketplace,
		notice_version: CONSENT_NOTICE_VERSION,
		granted_at: Date.UTC(2026, 8, 14),
		withdrawn_at: null,
		standing: true,
		...over
	};
}

function view(...consents: ConsentView[]): ConsentsView {
	return { notice_version: CONSENT_NOTICE_VERSION, consents };
}

describe('the permission rows', () => {
	it('lists every seller-device marketplace whatever the read did', () => {
		const unread = permissionRows(undefined);
		expect(unread.map((row) => row.marketplace)).toEqual([...SELLER_DEVICE]);
		expect(unread.every((row) => row.action === null && row.standing === 'unknown')).toBe(true);
	});

	it('offers Withdraw on a standing grant and Grant otherwise', () => {
		const rows = permissionRows(view(grant('Tpt')));
		const tpt = rows.find((row) => row.marketplace === 'Tpt');
		const tes = rows.find((row) => row.marketplace === 'Tes');
		expect(tpt?.action).toBe('withdraw');
		expect(tpt?.pill.label).toMatch(/^Granted /);
		expect(tpt?.pill.tone).toBe('ok');
		expect(tes?.action).toBe('grant');
		expect(tes?.pill).toEqual({ label: 'Not granted', tone: 'warn' });
	});

	it('asks for re-confirmation on a grant made against an older notice', () => {
		const rows = permissionRows(
			view(grant('Tpt', { notice_version: '2020-01-01', standing: false }))
		);
		const tpt = rows.find((row) => row.marketplace === 'Tpt');
		expect(tpt?.standing).toBe('outdated');
		expect(tpt?.pill).toEqual({ label: 'Needs re-confirmation', tone: 'warn' });
		expect(tpt?.action).toBe('grant');
	});
});

describe('the withdrawal prompt', () => {
	it('says what stops and what does not', () => {
		const prompt = withdrawPrompt('TPT');
		expect(prompt).toContain('Withdraw permission for TPT?');
		expect(prompt).toContain('stops starting new TPT work on every machine');
		expect(prompt).toContain('Logins already on your machines stay');
	});
});

describe('the record', () => {
	it('lists grants and withdrawals newest first', () => {
		const lines = historyLines([
			grant('Tpt', { granted_at: 1_000, withdrawn_at: 3_000, standing: false }),
			grant('Tes', { granted_at: 2_000 })
		]);
		expect(lines.map((line) => line.text)).toEqual([
			expect.stringContaining('TPT permission withdrawn'),
			expect.stringContaining('TES permission granted'),
			expect.stringContaining('TPT permission granted')
		]);
		expect(new Set(lines.map((line) => line.key)).size).toBe(3);
	});
});
