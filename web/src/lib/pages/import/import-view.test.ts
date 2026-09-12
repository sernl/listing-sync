import { describe, expect, it } from 'vitest';
import {
	NEEDS_THE_APP,
	NOTHING_CONNECTED,
	connectionUnknown,
	deviceLine,
	importBlocked,
	importCards,
	importLabel,
	importRows,
	importSitesOn,
	notConnected,
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

describe('importBlocked', () => {
	it('lets a connected, readable marketplace be imported from', () => {
		expect(importBlocked(cardFor('Tes', [connection()]), null)).toBeNull();
	});

	it('refuses a marketplace nothing reads before anything else', () => {
		const etsy = importCards([]).find((card) => card.unreadable !== null);
		expect(etsy).toBeUndefined();
	});

	// The plan's own refusal outranks the connection: a seller whose plan does
	// not cover reading a shop is told that rather than being sent to connect
	// one they would still not be able to read.
	it('states the plan’s refusal ahead of an absent connection', () => {
		const refused = 'Your plan does not cover reading a shop.';
		expect(importBlocked(cardFor('Tes'), refused)).toBe(refused);
	});

	it('names the absent connection', () => {
		const tes = cardFor('Tes');
		expect(importBlocked(tes, null)).toBe(notConnected(tes));
	});

	// An unread list is not a seller with no marketplace, and the card must not
	// send them to repair a connection that may be perfectly healthy.
	it('says it does not know rather than that the shop is disconnected', () => {
		const tes = cardFor('Tes', null);
		const blocked = importBlocked(tes, null);
		expect(blocked).toBe(connectionUnknown(tes));
		expect(blocked).not.toBe(notConnected(tes));
	});

	it('states the unknown as ours rather than as a disconnection', () => {
		const said = connectionUnknown(cardFor('Tes', null));
		expect(said).toContain('could not read');
		expect(said).not.toContain('is not connected');
	});

	// The declaration is asked once at connect time, on Marketplaces, so
	// nothing here gates on it.
	it('does not gate on the authorship declaration', () => {
		expect(
			importBlocked(cardFor('Tes', [connection({ authorship: undefined })]), null)
		).toBeNull();
	});
});

describe('the card’s own sentences', () => {
	it('names the marketplace in the control and in both permanent lines', () => {
		const tes = cardFor('Tes');
		expect(importLabel(tes)).toBe('Import from TES');
		expect(notConnected(tes)).toContain('TES');
		expect(deviceLine(tes)).toContain('TES');
	});

	// The reading happens under a login held on one machine, so the line has
	// to say the app must be open: a seller who presses Import in a browser
	// and closes the tab otherwise has no way to know why nothing happened.
	it('says where the reading happens and what has to be open', () => {
		expect(deviceLine(cardFor('Tes'))).toContain('Teachouse app');
		expect(deviceLine(cardFor('Tes'))).toContain('open');
		expect(NEEDS_THE_APP).toContain('Teachouse app');
		expect(NEEDS_THE_APP).toContain('login');
	});

	// The founder's own reading of this screen: "I can't see any option to
	// connect to the TPT/TES, how is that supposed to work?" Naming the screen
	// alone was what produced it, because that screen sent them back to the
	// downloads.
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

	it('carries the run state’s own word and tone', () => {
		const row = importRows([run({ state: 'reviewing', counts: counts({ review: 2 }) })])[0];
		expect(row?.label).toBe('Needs you');
		expect(row?.tone).toBe('warn');
	});

	it('counts what the run did', () => {
		const row = importRows([
			run({
				read_total: 9,
				counts: counts({ imported: 6, skipped: 2, failed: 1 })
			})
		])[0];
		expect(row?.line).toBe('9 found · 6 imported · 2 left out · 1 with problems');
	});

	// A run whose enumeration has not landed has no denominator, and a zero
	// there would read as a shop with nothing in it.
	it('says a shop is being read rather than inventing a total', () => {
		const row = importRows([run({ state: 'reading', read_total: null, counts: counts() })])[0];
		expect(row?.line).toBe('Reading what is in your shop.');
	});
});
