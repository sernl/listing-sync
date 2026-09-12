import { describe, expect, it } from 'vitest';
import {
	importBlocked,
	importCards,
	importRows,
	importSitesOn,
	pillTone,
	standingBadge,
	startRefusal,
	type ImportCard
} from './import-view';
import { APP_TOO_OLD } from '$lib/desktop';
import { TRANSPORT_OF } from '$lib/inventory';
import { AUTHORABLE_PLATFORMS } from '$lib/platforms';
import type { ConnectionView, ImportRunCounts, ImportRunHead } from '$lib/api';
import type { Marketplace } from '$lib/generated/vocab';

function connection(partial: Partial<ConnectionView> = {}): ConnectionView {
	return {
		id: 'c-1',
		marketplace: 'Tes',
		state: 'linked',
		status: 'connected',
		created_at: 0,
		updated_at: 0,
		...partial
	};
}

function counts(partial: Partial<ImportRunCounts> = {}): ImportRunCounts {
	return {
		listed: 0,
		selected: 0,
		read: 0,
		matched: 0,
		review: 0,
		imported: 0,
		skipped: 0,
		failed: 0,
		...partial
	};
}

function run(partial: Partial<ImportRunHead> = {}): ImportRunHead {
	return {
		id: 'run-1',
		kind: 'marketplace',
		source: 'Tes',
		batch_id: null,
		state: 'complete',
		read_total: 4,
		counts: counts({ imported: 4 }),
		created_at: 2,
		settled_at: 3,
		retry_of: null,
		execution: {
			owner_device: null, attempt: 1, lease_expires_at: null,
			last_contact_at: null, last_progress_at: null, stage: 'completed',
			reason_code: null, reason: null, discovered: 4, processed: 4, described: 4,
			enumeration_complete: true, selected_total: 4, commit_authorised: true
		},
		...partial
	};
}

function cardFor(marketplace: Marketplace, connections: ConnectionView[] | null = []): ImportCard {
	const card = importCards(connections).find((entry) => entry.marketplace === marketplace);
	if (card === undefined) {
		throw new Error(`${marketplace} is not on the import screen`);
	}
	return card;
}

const MARKETPLACES = Object.keys(TRANSPORT_OF) as Marketplace[];

describe('importCards', () => {
	it('shows only the marketplaces whose work runs on the seller’s own device', () => {
		const shown = importCards([]).map((card) => card.marketplace);
		for (const marketplace of MARKETPLACES) {
			expect(shown.includes(marketplace)).toBe(TRANSPORT_OF[marketplace] === 'SellerDevice');
		}
	});

	// An import reads what describes a listing rather than downloading its
	// file, so it is not gated on the file-download capture a migration needs:
	// both shops the app can sign into are readable, and a TPT card that still
	// said "nothing here reads a TPT shop" would be refusing a thing that now
	// works.
	it('offers both shops the app can sign into', () => {
		for (const marketplace of ['Tes', 'Tpt'] as const) {
			const card = cardFor(marketplace);
			expect(card.unreadable, marketplace).toBeNull();
			expect(card.sites, marketplace).toEqual(importSitesOn(marketplace));
			expect(card.sites.length, marketplace).toBeGreaterThan(0);
		}
	});

	it('reads a shop from every site this console can author on', () => {
		const offered = importCards([]).flatMap((card) => card.sites);
		expect([...offered].sort()).toEqual([...AUTHORABLE_PLATFORMS].sort());
	});

	// A card with no site and no sentence would render as a marketplace that
	// silently does nothing, which is the one outcome this screen must not
	// have. The type cannot hold this, because a readable marketplace
	// legitimately carries no reason, so the test does.
	it('gives every marketplace it cannot read a stated reason', () => {
		for (const card of importCards([])) {
			if (card.sites.length === 0) {
				expect(card.unreadable).toBeTruthy();
			} else {
				expect(card.unreadable).toBeNull();
			}
		}
	});

	it('reads a connection as standing rather than as health', () => {
		expect(
			cardFor('Tes', [connection({ state: 'needs_reauth', status: 'disconnected' })]).standing
		).toBe('held');
		expect(cardFor('Tes', [connection({ marketplace: 'Tpt' })]).standing).toBe('absent');
	});

	// The defect this replaces: the card read presence alone, so a marketplace
	// the seller had just disconnected went on wearing the green Connected
	// badge while the marketplaces screen said it was off.
	it('does not badge a disconnected or revoked marketplace as connected', () => {
		for (const state of ['unlinked', 'revoked']) {
			const card = cardFor('Tes', [connection({ state, status: 'disconnected' })]);
			expect(card.standing, state).toBe('absent');
		}
	});

	// The failure this type exists to prevent: an unreadable connections list
	// rendering as a marketplace the seller has not connected, which sends them
	// to repair something that is not broken.
	it('holds an unread list apart from a seller with no marketplace', () => {
		expect(cardFor('Tes', null).standing).toBe('unread');
		expect(cardFor('Tes', []).standing).toBe('absent');
	});

	it('never renders an unread list as a claim about the connection', () => {
		const unread = standingBadge(cardFor('Tes', null));
		expect(unread.label).not.toBe(standingBadge(cardFor('Tes', [])).label);
		expect(unread.tone).toBe('soon');
	});
});

describe('importBlocked', () => {
	it('lets a browser queue work without claiming a local login', () => {
		expect(importBlocked(cardFor('Tes', [connection()]), null, undefined, false)).toBeNull();
	});
	it('does not treat another device’s recorded connection as a local login', () => {
		expect(importBlocked(cardFor('Tes', [connection()]), null, {
			kind: 'known', connected: false
		}, true)).not.toBeNull();
	});

	it('uses the local login rather than stale server standing', () => {
		expect(importBlocked(cardFor('Tes', []), null, {
			kind: 'known', connected: true
		}, true)).toBeNull();
	});

	it('preserves unknown local availability rather than reporting an absent login', () => {
		const card = cardFor('Tes', [connection()]);
		expect(importBlocked(card, null, { kind: 'unavailable' }, true)).not.toBe(
			importBlocked(card, null, { kind: 'known', connected: false }, true)
		);
	});

	it('keeps entitlement refusal ahead of local session availability', () => {
		expect(importBlocked(cardFor('Tes'), 'This plan does not cover imports.', undefined, true))
			.toBe('This plan does not cover imports.');
	});
});


describe('startRefusal', () => {
	it('says nothing where the work started', () => {
		expect(startRefusal({ kind: 'started' })).toBeNull();
	});

	// A browser was never going to run the pass, so there is no failure to
	// report: the run exists and the page says where the work happens.
	it('says nothing in a browser', () => {
		expect(startRefusal({ kind: 'unavailable' })).toBeNull();
	});

	it('renders the application’s own words unaltered', () => {
		expect(startRefusal({ kind: 'refused', detail: 'No session for Tes.' })).toBe(
			'No session for Tes.'
		);
	});

	it('substitutes the update sentence, which is a remedy rather than a refusal', () => {
		expect(startRefusal({ kind: 'unsupported' })).toBe(APP_TOO_OLD);
	});
});

describe('pillTone', () => {
	// The badge and the stage model spell the same four tones, and only grey
	// differently. Pinned in full because the request page reads this map for
	// every listing row as well as for the stage itself.
	it('carries every stage tone into the badge', () => {
		expect(pillTone('ok')).toBe('ok');
		expect(pillTone('run')).toBe('run');
		expect(pillTone('bad')).toBe('bad');
		expect(pillTone('mut')).toBe('soon');
	});
});

describe('importRows', () => {
	// One list for both ways in. A seller who read a shop on Monday and a
	// spreadsheet on Tuesday has made two imports, not one of each: the two
	// tables behind them are ours rather than theirs.
	it('lists both kinds of import in the endpoint’s own order', () => {
		const rows = importRows([
			run({ id: 'shop' }),
			run({ id: 'sheet', kind: 'spreadsheet', source: null, batch_id: 'b-9' })
		]);
		expect(rows.map((row) => row.id)).toEqual(['shop', 'sheet']);
	});

	// A spreadsheet run is read on its batch's own page, which holds the
	// report its rows came from; a second screen for it would show the review
	// without the rows it is about.
	it('sends a spreadsheet run to its batch and a shop run to its own page', () => {
		const [shop, sheet] = importRows([
			run({ id: 'shop' }),
			run({ id: 'sheet', kind: 'spreadsheet', source: null, batch_id: 'b-9' })
		]);
		expect(shop?.href).toBe('/imports/runs/shop');
		expect(sheet?.href).toBe('/imports/b-9');
	});

	it('names a shop run by its shop and the other by the way it came in', () => {
		const [shop, sheet] = importRows([
			run(),
			run({ kind: 'spreadsheet', source: null, batch_id: 'b-9' })
		]);
		expect(shop?.source).toBe('Tes');
		expect(sheet?.source).toBeNull();
		expect(sheet?.name).toBe('Spreadsheet');
	});

});
