// What one marketplace tile says and where tapping it goes.
//
// The status half is asserted against `STATE_LABEL` and `STATE_TONE` rather
// than against words written here, because the point of the tile is that it
// does not hold a second status vocabulary: a word changed in `inventory.ts`
// must move the tile with it rather than leaving the two disagreeing.

import { describe, expect, it } from 'vitest';
import {
	STATE_LABEL,
	STATE_TONE,
	chipFor,
	type ChipInput,
	type MarketplaceChip,
	type MarketplaceState,
	type WorkItem
} from '$lib/inventory';
import { MARK_SRC, platformTitle } from '$lib/platforms';
import type { ConnectionView, ItemView, MappingHead } from '$lib/api';
import type { Readiness } from '$lib/publish-readiness';
import type { InventoryId } from '$lib/generated/vocab';
import { tileFor, type TileInput } from './marketplace-tile';

const NOW = 1_788_000_000_000;
const PRODUCT = 'p-fractions';

function mapping(partial: Partial<MappingHead> & { inventory: InventoryId }): MappingHead {
	return {
		id: `m-${partial.inventory}`,
		product: PRODUCT,
		binding_state: 'bound',
		lifecycle_state: 'live',
		updated_at: NOW - 3 * 24 * 60 * 60 * 1000,
		listing_url: null,
		...partial
	};
}

function connection(inventory: InventoryId): ConnectionView {
	return {
		id: 'c1',
		marketplace: inventory === 'Tpt' ? 'Tpt' : inventory === 'Etsy' ? 'Etsy' : 'Tes',
		state: 'linked',
		status: 'connected',
		created_at: 1,
		updated_at: 2
	};
}

function work(partial: Partial<ItemView>, job = 'j1'): WorkItem {
	return {
		job,
		item: {
			item: 'i1',
			mapping: 'm-Tes',
			state: 'queued',
			attempt_count: 1,
			created_at: NOW,
			...partial
		}
	};
}

function chip(partial: Partial<ChipInput> & { inventory: InventoryId }): MarketplaceChip {
	return chipFor({
		product: PRODUCT,
		mapping: mapping({ inventory: partial.inventory }),
		work: undefined,
		connection: connection(partial.inventory),
		status: undefined,
		...partial
	});
}

function tile(partial: Partial<TileInput> & { chip: MarketplaceChip }) {
	return tileFor({
		mapping: mapping({ inventory: partial.chip.inventory }),
		product: PRODUCT,
		now: NOW,
		readiness: undefined,
		...partial
	});
}

/** One chip per state in the closed vocabulary, each reached the way the
 *  console reaches it rather than by writing the state in by hand. */
const BY_STATE: Record<MarketplaceState, MarketplaceChip> = {
	listed: chip({ inventory: 'Tes' }),
	draft: chip({
		inventory: 'Tes',
		mapping: mapping({ inventory: 'Tes', lifecycle_state: 'draft' })
	}),
	not_listed: chip({
		inventory: 'Tes',
		mapping: mapping({ inventory: 'Tes', binding_state: 'unbound', lifecycle_state: 'absent' })
	}),
	in_flight: chip({ inventory: 'Tes', work: work({ state: 'running' }) }),
	blocked: chip({
		inventory: 'Tes',
		work: work({ state: 'blocked', blocked_on: 'election' })
	}),
	needs_signin: chip({ inventory: 'Tes', connection: undefined }),
	stranded: chip({ inventory: 'Tes', work: work({ state: 'parked_live' }) }),
	failed: chip({
		inventory: 'Tes',
		work: work({
			state: 'settled',
			outcome: 'failed',
			failure_detail:
				'The marketplace rejected the upload because the file exceeded its own size limit. Nothing was published.'
		})
	})
};

describe('one tile per state in the closed vocabulary', () => {
	it('reaches all eight of them, so the sweep below is not vacuous', () => {
		const states = Object.keys(BY_STATE) as MarketplaceState[];
		expect(states.length).toBe(Object.keys(STATE_LABEL).length);
		for (const state of states) {
			expect(BY_STATE[state].state, state).toBe(state);
		}
	});

	it('takes its word and its tone from the board, never from a second table', () => {
		for (const [state, held] of Object.entries(BY_STATE)) {
			const drawn = tile({ chip: held });
			expect(drawn.label, state).toBe(STATE_LABEL[state as MarketplaceState]);
			expect(drawn.tone, state).toBe(STATE_TONE[state as MarketplaceState]);
		}
	});

	// A clause and not a sentence. The full stop is what the assertion tests
	// for, because the failure being guarded is the chip's own sentence being
	// passed through: every one of those ends in one, and every one of them is
	// too long for the two lines a tile has.
	it('says one short clause under the word, with no sentence in it', () => {
		for (const [state, held] of Object.entries(BY_STATE)) {
			const drawn = tile({ chip: held });
			expect(drawn.detail.trim().length, state).toBeGreaterThan(0);
			expect(drawn.detail, state).not.toContain('.');
			expect(drawn.detail.length, state).toBeLessThanOrEqual(60);
		}
	});

	it('spells the marketplace and the status out in the accessible name', () => {
		for (const [state, held] of Object.entries(BY_STATE)) {
			const drawn = tile({ chip: held });
			expect(drawn.name.startsWith(platformTitle('Tes')), state).toBe(true);
			expect(drawn.name, state).toContain(STATE_LABEL[state as MarketplaceState]);
		}
	});

	it('draws the marketplace by its own mark', () => {
		expect(tile({ chip: BY_STATE.listed }).markSrc).toBe(MARK_SRC.Tes);
		const tpt = tile({
			chip: chip({ inventory: 'Tpt' }),
			mapping: mapping({ inventory: 'Tpt' })
		});
		expect(tpt.markSrc).toBe(MARK_SRC.Tpt);
	});
});

describe('where tapping a tile goes', () => {
	it('sends a marketplace needing a sign-in to its own card, not to the bare page', () => {
		const drawn = tile({
			chip: chip({ inventory: 'Tes', connection: undefined }),
			mapping: mapping({ inventory: 'Tes' })
		});
		expect(drawn.href).toBe('/marketplaces#mp-tes');
		expect(drawn.external).toBe(false);
		const tpt = tile({
			chip: chip({ inventory: 'Tpt', connection: undefined }),
			mapping: mapping({ inventory: 'Tpt' })
		});
		expect(tpt.href).toBe('/marketplaces#mp-tpt');
	});

	it('opens a live listing on the marketplace, in the seller’s own browser', () => {
		const listed = mapping({ inventory: 'Tes', listing_url: 'https://www.tes.com/x' });
		const drawn = tile({
			chip: chip({ inventory: 'Tes', mapping: listed }),
			mapping: listed
		});
		expect(drawn.href).toBe('https://www.tes.com/x');
		expect(drawn.external).toBe(true);
	});

	// The severe one. The obvious implementation passes `chip.action` straight
	// through, and every state whose action is "open the item" then draws a
	// link back to the page the tile is already on: four tiles that look like
	// destinations and go nowhere. It fails under exactly that implementation.
	it('drops a link that points at the page the tile is drawn on', () => {
		const noUrl = mapping({ inventory: 'Tes', listing_url: null });
		const listed = chip({ inventory: 'Tes', mapping: noUrl });
		expect(listed.action?.href).toBe(`/resources/${PRODUCT}`);
		const drawn = tile({ chip: listed, mapping: noUrl });
		expect(drawn.href).toBe(null);
		expect(drawn.external).toBe(false);

		for (const state of ['draft', 'not_listed'] as const) {
			expect(tile({ chip: BY_STATE[state] }).href, state).toBe(null);
		}
	});

	it('keeps a link to another resource, which is not this page', () => {
		const elsewhere = chip({ inventory: 'Tes', product: 'p-other' });
		const drawn = tile({ chip: elsewhere, product: PRODUCT });
		expect(drawn.href).toBe('/resources/p-other');
	});

	it('sends every state a run drives to that run', () => {
		for (const state of ['in_flight', 'stranded', 'failed'] as const) {
			expect(tile({ chip: BY_STATE[state] }).href, state).toBe('/sync/j1');
		}
	});

	it('offers nowhere to go for a marketplace this resource does not reach', () => {
		const drawn = tile({
			chip: chip({ inventory: 'Tpt', mapping: undefined }),
			mapping: undefined
		});
		expect(drawn.href).toBe(null);
	});
});

describe('the control under the status', () => {
	it('offers to record a listing the console has never bound', () => {
		const unbound = mapping({
			inventory: 'Tes',
			binding_state: 'unbound',
			lifecycle_state: 'absent'
		});
		const drawn = tile({ chip: chip({ inventory: 'Tes', mapping: unbound }), mapping: unbound });
		expect(drawn.control).toBe('attach');
		expect(drawn.mapping).toBe('m-Tes');
	});

	it('offers to cross-list onto a marketplace this console can author for', () => {
		const drawn = tile({
			chip: chip({ inventory: 'Tpt', mapping: undefined }),
			mapping: undefined
		});
		expect(drawn.control).toBe('crosslist');
		expect(drawn.mapping).toBe(null);
	});

	// Etsy has no create path at all, so offering to add it would let a seller
	// map a platform nothing can ever send to.
	it('offers nothing for a marketplace this console cannot author for', () => {
		const drawn = tile({
			chip: chip({ inventory: 'Etsy', mapping: undefined }),
			mapping: undefined
		});
		expect(drawn.control).toBe(null);
	});

	it('offers nothing where the listing is already bound', () => {
		expect(tile({ chip: BY_STATE.listed }).control).toBe(null);
	});
});

describe('what the tile no longer draws but still says', () => {
	const refused: Readiness = {
		inventory: 'Tes',
		title: platformTitle('Tes'),
		ready: false,
		line: 'needs a licence',
		tone: 'run'
	};

	it('carries the readiness verdict into the accessible name', () => {
		const drawn = tile({ chip: BY_STATE.listed, readiness: refused });
		expect(drawn.name).toContain('Next send: needs a licence.');
	});

	it('says nothing about the next send where nothing refuses it', () => {
		const drawn = tile({
			chip: BY_STATE.listed,
			readiness: { ...refused, ready: true, line: 'ready to send', tone: 'ok' }
		});
		expect(drawn.name).not.toContain('Next send');
	});

	it('carries a paused marketplace as an overlay rather than as a ninth state', () => {
		const held = chip({
			inventory: 'Tes',
			status: { inventory: 'Tes', marketplace: 'Tes', halted: true, raised_at: 1, reason: 'a Tes outage' }
		});
		const drawn = tile({ chip: held });
		expect(drawn.paused).toBe('a Tes outage');
		expect(drawn.label).toBe(STATE_LABEL.listed);
		expect(drawn.name).toContain('Sending is paused here: a Tes outage');
	});
});
