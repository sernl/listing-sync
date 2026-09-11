import { describe, expect, it } from 'vitest';
import {
	IMPORT_IS_A_MIGRATION,
	MIGRATION_HREF,
	connectionUnknown,
	pillTone,
	deviceLine,
	handoffBlocked,
	importCards,
	importRows,
	NOTHING_CONNECTED,
	notConnected,
	standingBadge,
	startRefusal,
	type ImportCard
} from './import-view';
import { APP_TOO_OLD } from '$lib/desktop';
import { MIGRATE_SOURCES, sourcesOn } from '$lib/sync-request';
import { TRANSPORT_OF } from '$lib/inventory';
import type { ConnectionView, SyncRequestHead } from '$lib/api';
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

function head(partial: Partial<SyncRequestHead> = {}): SyncRequestHead {
	return {
		request: 'r-1',
		source: 'Tes',
		target: 'Tpt',
		disposition: 'migrate',
		intent: 'draft',
		state: 'enqueued',
		created_at: 1,
		resources_total: 3,
		resources_failed: 0,
		...partial
	};
}

function cardFor(
	marketplace: Marketplace,
	connections: ConnectionView[] | null = []
): ImportCard {
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

	it('puts the marketplace a shop can be read from first', () => {
		const cards = importCards([]);
		expect(cards[0]?.marketplace).toBe('Tes');
		expect(cards[0]?.unreadable).toBeNull();
		expect(cards.at(-1)?.unreadable).not.toBeNull();
	});

	it('offers every source the server admits', () => {
		const tes = cardFor('Tes');
		expect(tes.sites).toEqual(sourcesOn('Tes'));
		expect(tes.sites).toEqual(MIGRATE_SOURCES);
	});

	// A card with no site and no sentence would render as a marketplace that
	// silently does nothing, which is the one outcome this screen must not
	// have. The type cannot hold this, because `Tes` legitimately carries no
	// reason, so the test does.
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
			expect(standingBadge(card).label, state).toBe('Not connected');
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
		expect(unread.label).toBe('Not known');
	});
});

describe('handoffBlocked', () => {
	it('lets a connected, readable marketplace hand off', () => {
		expect(handoffBlocked(cardFor('Tes', [connection()]))).toBeNull();
	});

	it('refuses a marketplace nothing reads before anything else', () => {
		const tpt = cardFor('Tpt', [connection({ marketplace: 'Tpt' })]);
		expect(handoffBlocked(tpt)).toBe(tpt.unreadable);
	});

	it('names the absent connection', () => {
		const tes = cardFor('Tes');
		expect(handoffBlocked(tes)).toBe(notConnected(tes));
	});

	// An unread list is not a seller with no marketplace, and the card must not
	// send them to repair a connection that may be perfectly healthy.
	it('says it does not know rather than that the shop is disconnected', () => {
		const tes = cardFor('Tes', null);
		const blocked = handoffBlocked(tes);
		expect(blocked).toBe(connectionUnknown(tes));
		expect(blocked).not.toBe(notConnected(tes));
	});

	it('states the unknown as ours rather than as a disconnection', () => {
		const said = connectionUnknown(cardFor('Tes', null));
		expect(said).toContain('could not read');
		expect(said).not.toContain('is not connected');
	});

	// D8 gates the declaration where the request is raised, so nothing here
	// asks about it: a connected, readable marketplace hands off whether or not
	// this console holds a declaration for the target.
	it('does not gate on the authorship declaration', () => {
		expect(handoffBlocked(cardFor('Tes', [connection({ authorship: undefined })]))).toBeNull();
	});
});

describe('the handoff destination', () => {
	it('points at the screen that owns raising the request', () => {
		expect(MIGRATION_HREF).toBe('/automations/migration');
	});
});

describe('the card’s own sentences', () => {
	it('names the marketplace in both permanent lines', () => {
		const tes = cardFor('Tes');
		expect(notConnected(tes)).toContain('TES');
		expect(deviceLine(tes)).toContain('TES');
		expect(deviceLine(tes)).toContain('checks in');
	});

	it('says what a migration does with the listings it finds', () => {
		expect(IMPORT_IS_A_MIGRATION).toContain('TPT draft');
		expect(IMPORT_IS_A_MIGRATION).toContain('coming');
	});

	// The founder's own reading of this screen: "I can't see any option to
	// connect to the TPT/TES, how is that supposed to work?" Naming the screen
	// alone was what produced it, because that screen sent them back to the
	// downloads. Both sentences now name the app and say the login stays on the
	// machine, which is the fact that makes the app the only place it happens.
	it('says where a connection is actually made, not only which screen to open', () => {
		for (const sentence of [notConnected(cardFor('Tes')), NOTHING_CONNECTED]) {
			expect(sentence).toContain('Teachouse app');
			expect(sentence).toContain('Marketplaces');
		}
		expect(notConnected(cardFor('Tes'))).toContain('on that machine');
		expect(NOTHING_CONNECTED).toContain('stays on that machine');
	});
});

describe('startRefusal', () => {
	it('says nothing where the work started', () => {
		expect(startRefusal({ kind: 'started' })).toBeNull();
	});

	// A browser was never going to run the pass, so there is no failure to
	// report: the request exists and its own page says where the work happens.
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
	it('keeps imports and drops the syncs the same endpoint serves', () => {
		const rows = importRows([head(), head({ request: 'r-2', disposition: 'sync' })]);
		expect(rows.map((row) => row.request)).toEqual(['r-1']);
	});

	it('keeps the endpoint’s own order, which is newest first', () => {
		const rows = importRows(
			[head({ request: 'new', created_at: 2 }), head({ request: 'old', created_at: 1 })]);
		expect(rows.map((row) => row.request)).toEqual(['new', 'old']);
	});

	it('names both ends of the move', () => {
		expect(importRows([head()])[0]).toMatchObject({ source: 'Tes', target: 'Tpt' });
	});

	it('carries the stage’s own word and tone', () => {
		const row = importRows([head({ state: 'draining', resources_total: 2 })])[0];
		expect(row?.label).toBe('Importing');
		expect(row?.tone).toBe('run');
	});

	// The one tone the badge spells differently from the stage model: a stage
	// that is nothing to act on is grey, and the badge calls that grey `soon`.
	it('renders a waiting import in the badge’s grey', () => {
		const row = importRows([head({ state: 'pending', resources_total: 0 })])[0];
		expect(row?.tone).toBe('soon');
		expect(row?.line).toContain('Waiting for your device');
	});

	// The row reads the headline alone, so the count has to be in the headline:
	// carried in the detail it would never reach a list row. Amber rather than
	// red, because a wholly skipped import is a request that completed and
	// achieved nothing rather than one that broke.
	it('reads a wholly skipped import as nothing imported, with the count', () => {
		const row = importRows(
			[head({ state: 'enqueued', resources_total: 3, resources_failed: 3 })])[0];
		expect(row?.label).toBe('Nothing imported');
		expect(row?.line).toBe('Nothing was imported: 3 listings skipped.');
		expect(row?.tone).toBe('run');
		expect(row?.line).toContain('skipped');
	});

	// One skipped listing is the commonest real shape of the case above, and
	// the count is composed rather than interpolated, so the singular is worth
	// pinning from the side that reads it.
	it('counts one skipped listing in the singular', () => {
		const row = importRows(
			[head({ state: 'enqueued', resources_total: 1, resources_failed: 1 })])[0];
		expect(row?.line).toBe('Nothing was imported: 1 listing skipped.');
	});

	it('counts a partly skipped import by what arrived', () => {
		const row = importRows(
			[head({ state: 'enqueued', resources_total: 5, resources_failed: 2 })])[0];
		expect(row?.line).toBe('3 listings imported.');
		expect(row?.tone).toBe('run');
	});

	it('says a completed import differently from an empty shop', () => {
		const done = importRows([head({ state: 'enqueued', resources_total: 3 })])[0];
		const empty = importRows([head({ state: 'enqueued', resources_total: 0 })])[0];
		expect(done?.label).toBe('Imported');
		expect(empty?.label).toBe('Nothing to import');
	});
});
