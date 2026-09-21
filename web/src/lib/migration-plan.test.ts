import { describe, expect, it } from 'vitest';
import type { MigrationCap } from './api';
import {
	DISPOSITIONS,
	DISPOSITION_LINE,
	DISPOSITION_WORD,
	MIGRATION_SOURCES,
	VERDICT_WORD,
	capSentence,
	confirmLabel,
	countsLine,
	migrationBody,
	migrationSources,
	migrationTargets,
	pairReason,
	productsFromUrl
} from './migration-plan';

const CAP: MigrationCap = {
	available: 12,
	required: 8,
	remaining: 4
};

describe('the pair table', () => {
	it('offers every marketplace at both ends rather than hiding the refused ones', () => {
		// A marketplace left out reads as one Teachouse has never heard of, which
		// is a different and false statement from "it cannot be a source yet".
		expect(migrationSources().map((side) => side.inventory)).toEqual(['Tpt', 'Tes', 'Etsy']);
		expect(migrationTargets().map((side) => side.inventory)).toEqual(['Tpt', 'Tes', 'Etsy']);
	});

	it('allows captured sources and leaves Etsy unavailable', () => {
		const refused = migrationSources()
			.filter((side) => !side.enabled)
			.map((side) => side.inventory);
		expect(refused).toEqual(['Etsy']);
		expect(MIGRATION_SOURCES).toEqual(['Tpt', 'Tes']);
	});

	it('refuses Etsy as a target because nothing writes to it yet', () => {
		const etsy = migrationTargets().find((side) => side.inventory === 'Etsy');
		expect(etsy?.enabled).toBe(false);
		expect(etsy?.reason).toBe('Etsy is not connected to Teachouse yet.');
		expect(migrationTargets().find((side) => side.inventory === 'Tes')?.enabled).toBe(true);
	});

	it('allows both directions between TES and TPT', () => {
		const pairs: string[] = [];
		for (const source of migrationSources()) {
			for (const target of migrationTargets()) {
				if (pairReason(source.inventory, target.inventory) === null) {
					pairs.push(`${source.inventory}->${target.inventory}`);
				}
			}
		}
		expect(pairs).toEqual(['Tpt->Tes', 'Tes->Tpt']);
	});

	it('refuses a pair whose two ends are the same marketplace, and says which', () => {
		expect(pairReason('Tes', 'Tes')).toContain('both ends here are TES');
	});

	// The sameness slip is the seller's own and is fixed by changing a select;
	// a capture gap is not theirs to fix at all. Reading the second when the
	// first is true sends them looking for a setting that does not exist.
	it('names the sameness before a capture gap when both hold', () => {
		expect(pairReason('Etsy', 'Etsy')).toContain('both ends here are Etsy');
		expect(pairReason('Etsy', 'Tpt')).toContain('cannot yet download');
		expect(pairReason('Tes', 'Etsy')).toContain('not connected to Teachouse yet');
	});
});

describe('the words for a migration', () => {
	it('calls the two dispositions Copy and Move, in that order', () => {
		expect(DISPOSITIONS.map((disposition) => DISPOSITION_WORD[disposition])).toEqual([
			'Copy',
			'Move'
		]);
	});

	it('says of each what becomes of the source listing, which is the difference', () => {
		expect(DISPOSITION_LINE.sync).toContain('leaves the source listing');
		expect(DISPOSITION_LINE.migrate).toContain('removes it from the source');
	});

	it('words the three verdicts as a seller reads them', () => {
		expect(VERDICT_WORD).toEqual({
			will_create: 'Will create',
			already_there: 'Already there',
			blocked: 'Blocked'
		});
	});

	it('agrees the confirm label with its count', () => {
		expect(confirmLabel('sync', 1)).toBe('Copy 1 resource');
		expect(confirmLabel('migrate', 8)).toBe('Move 8 resources');
	});

	it('prints every count, zeroes included', () => {
		expect(countsLine({ will_create: 8, already_there: 3, blocked: 1 })).toBe(
			'8 will be created, 3 already there, 1 blocked'
		);
		expect(countsLine({ will_create: 0, already_there: 0, blocked: 0 })).toBe(
			'0 will be created, 0 already there, 0 blocked'
		);
	});
});

describe('the cap sentence', () => {
	it('states the balance and what the selection would spend', () => {
		expect(capSentence(CAP, 8)).toEqual({
			line: 'You have 12 moves. This uses 8.',
			refusal: null
		});
	});

	it('leaves out the selection clause where nothing is selected yet', () => {
		expect(capSentence(CAP, 0).line).toBe('You have 12 moves.');
		expect(capSentence(CAP, 0).refusal).toBeNull();
	});

	it('allows a selection that spends the balance exactly', () => {
		expect(capSentence(CAP, 12).refusal).toBeNull();
	});

	it('refuses one resource past the balance, saying how many would fit', () => {
		const over = capSentence(CAP, 13);
		expect(over.refusal).toBe('You have 12 moves and this needs 13. Buy a pack or pick fewer.');
		// The figure stays beside the refusal: a seller told they cannot press
		// still has to know how many would fit.
		expect(over.line).toContain('12 moves');
	});

	it('sends an empty balance to buy rather than to pick fewer', () => {
		const none = capSentence({ available: 0, required: 3, remaining: 0 }, 3);
		expect(none.line).toBe('You have no moves. This uses 3.');
		expect(none.refusal).toBe('You have no moves left. Buy a pack or choose Sync.');
	});

	// A balance another tab has spent would put the seller over without
	// warning, so the sentence is read off the plan's own block rather than
	// off the entitlement read.
	it('reads the figure it prints off the cap it was handed', () => {
		const spent = capSentence({ available: 1, required: 1, remaining: 0 }, 1);
		expect(spent.line).toBe('You have 1 move. This uses 1.');
		expect(spent.refusal).toBeNull();
	});
});

describe('a migration submit', () => {
	it('carries the whole source shop as a flag rather than as a list', () => {
		expect(
			migrationBody({
				source: 'Tes',
				target: 'Tpt',
				disposition: 'migrate',
				all: true,
				products: ['a', 'b']
			})
		).toEqual({
			source: 'Tes',
			target: 'Tpt',
			disposition: 'migrate',
			selection: { all: true }
		});
	});

	it('keeps an empty tick list a tick list', () => {
		const body = migrationBody({
			source: 'Tes',
			target: 'Tpt',
			disposition: 'sync',
			all: false,
			products: []
		});
		expect(body.selection).toEqual({ products: [] });
		expect(body.disposition).toBe('sync');
	});
});

describe('a preselection arriving in the address', () => {
	it('ticks what a comma-joined list names, in the order it names it', () => {
		expect(productsFromUrl(new URLSearchParams('products=a,b,c'))).toEqual(['a', 'b', 'c']);
	});

	it('drops blanks and repeats rather than refusing a hand-edited address', () => {
		expect(productsFromUrl(new URLSearchParams('products=a,,b,a&products= c '))).toEqual([
			'a',
			'b',
			'c'
		]);
	});

	it('preselects nothing where the address names nothing', () => {
		expect(productsFromUrl(new URLSearchParams(''))).toEqual([]);
		expect(productsFromUrl(new URLSearchParams('products='))).toEqual([]);
	});
});
