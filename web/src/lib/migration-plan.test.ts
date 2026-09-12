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
	limit: 20,
	used: 8,
	remaining: 12,
	// 1 October 2026, which is what the sentences below print.
	resets_at: Date.UTC(2026, 9, 1)
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
	it('states what is left, what the selection uses, and when the month turns', () => {
		expect(capSentence(CAP, 8)).toEqual({
			line: 'You have 12 of 20 moves left this month; this uses 8. Resets 1 October.',
			refusal: null
		});
	});

	it('leaves out the selection clause where nothing is selected yet', () => {
		expect(capSentence(CAP, 0).line).toBe(
			'You have 12 of 20 moves left this month. Resets 1 October.'
		);
		expect(capSentence(CAP, 0).refusal).toBeNull();
	});

	it('allows a selection that fills the allowance exactly', () => {
		expect(capSentence(CAP, 12).refusal).toBeNull();
	});

	it('refuses one resource past the allowance', () => {
		const over = capSentence(CAP, 13);
		expect(over.refusal).toBe(
			'Your plan moves 20 resources a month and you have 12 left, so 13 is more than this month can take. It resets 1 October.'
		);
		// The figures stay beside the refusal: a seller told they cannot press
		// still has to know how many would fit and when the rest can go.
		expect(over.line).toContain('12 of 20');
	});

	it('says a plan that moves nothing moves nothing, rather than counting to zero', () => {
		const none = capSentence({ limit: 0, used: 0, remaining: 0, resets_at: CAP.resets_at }, 3);
		expect(none.line).toBe('Your plan moves no resources between marketplaces.');
		expect(none.refusal).toContain('Upgrade');
	});

	// A server that counted a month the console did not would put the seller
	// over without warning, so the sentence is read off the plan's own block
	// rather than recomputed from the entitlement read.
	it('reads the figures it prints off the cap it was handed', () => {
		const spent = capSentence({ limit: 20, used: 20, remaining: 0, resets_at: CAP.resets_at }, 1);
		expect(spent.line).toContain('0 of 20');
		expect(spent.refusal).toContain('you have 0 left');
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
