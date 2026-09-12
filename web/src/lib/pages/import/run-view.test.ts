import { describe, expect, it } from 'vitest';
import {
	countsLine,
	emptyItemsLine,
	importedHref,
	inPlay,
	itemRows,
	progressLine,
	readSoFar,
	reviewCard,
	runBadge,
	runHref,
	runName,
	selectionBlocked,
	selectionRows,
	settledLine,
	stageCopy,
	stageFrom
} from './run-view';
import type {
	ImportRunCounts,
	ImportRunHead,
	ImportRunItemView,
	ImportRunView,
	ReviewPairView,
	ReviewSideView
} from '$lib/api';

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

function head(partial: Partial<ImportRunHead> = {}): ImportRunHead {
	return {
		id: 'run-1',
		kind: 'marketplace',
		source: 'Tes',
		batch_id: null,
		state: 'reading',
		read_total: null,
		counts: counts(),
		created_at: 1,
		settled_at: null,
		...partial
	};
}

function item(partial: Partial<ImportRunItemView> = {}): ImportRunItemView {
	return {
		state: 'listed',
		locator: 'https://shop/1',
		ordinal: 1,
		title: 'Fractions pack',
		price: { minor_units: 450, currency: 'Gbp' },
		cover_url: null,
		product_id: null,
		skip_reason: null,
		failure_detail: null,
		...partial
	};
}

function view(items: ImportRunItemView[], partial: Partial<ImportRunHead> = {}): ImportRunView {
	return { ...head(partial), items, review_pairs: [] };
}

function side(partial: Partial<ReviewSideView> = {}): ReviewSideView {
	return {
		product_id: 'p-1',
		run_locator: null,
		marketplace: 'Tes',
		title: 'Fractions pack',
		price: { minor_units: 450, currency: 'Gbp' },
		grades: ['Year 3', 'Year 4'],
		cover_url: null,
		...partial
	};
}

function pair(partial: Partial<ReviewPairView> = {}): ReviewPairView {
	return {
		product_lo: 'a',
		product_hi: 'b',
		sentence: 'The same file, byte for byte: worksheet-pack.pdf, 2.4 MB.',
		layer: 'l1',
		lo: side(),
		hi: side({
			product_id: null,
			run_locator: 'https://shop/1',
			marketplace: 'Tpt'
		}),
		...partial
	};
}

describe('stageFrom', () => {
	// The page for a run whose shop is still being listed, the page for a
	// seller choosing from it and the page for the reading are three different
	// pages, and the server's one `reading` state does not tell them apart.
	// `read_total` is what does: null until the enumeration lands.
	it('tells the three halves of "reading" apart by whether a total is known', () => {
		expect(stageFrom(head({ read_total: null }))).toBe('listing');
		expect(stageFrom(head({ read_total: 4, counts: counts({ listed: 4 }) }))).toBe('selecting');
		expect(stageFrom(head({ read_total: 4, counts: counts({ selected: 4 }) }))).toBe('reading');
	});

	// A total of nothing must not read as a shop with nothing in it, which is
	// the same reason the column is nullable: a run that has enumerated an
	// empty shop is past the listing step.
	it('does not read an enumerated empty shop as one still being listed', () => {
		expect(stageFrom(head({ read_total: 0, counts: counts() }))).toBe('reading');
	});

	it('carries the states the server already tells apart', () => {
		expect(stageFrom(head({ state: 'reviewing' }))).toBe('reviewing');
		expect(stageFrom(head({ state: 'committing' }))).toBe('committing');
		expect(stageFrom(head({ state: 'complete' }))).toBe('done');
	});

	// Given up is not finished, and a seller who gave up needs the page to say
	// what did land rather than to congratulate them.
	it('reads a failed and an abandoned run the same way, as unfinished', () => {
		expect(stageFrom(head({ state: 'failed' }))).toBe('failed');
		expect(stageFrom(head({ state: 'abandoned' }))).toBe('failed');
	});

	it('gives every stage a headline and a line under it', () => {
		for (const state of ['reading', 'reviewing', 'committing', 'complete', 'failed'] as const) {
			const copy = stageCopy(stageFrom(head({ state, read_total: 1 })));
			expect(copy.headline.length, state).toBeGreaterThan(0);
			expect(copy.detail.length, state).toBeGreaterThan(0);
		}
	});
});

describe('runBadge', () => {
	it('says a run wanting an answer differently from one under way', () => {
		expect(runBadge('reviewing')).toEqual({ tone: 'warn', label: 'Needs you' });
		expect(runBadge('reading').tone).toBe('run');
	});

	// A state added in Rust degrades to saying so rather than rendering an
	// unstyled blank, which is the one outcome a list of runs must not have.
	it('names an unrecognised state rather than rendering nothing', () => {
		expect(runBadge('teleporting')).toEqual({ tone: 'soon', label: 'Unknown' });
	});
});

describe('the live bar', () => {
	// A skip is mostly a resource the seller did not tick. Counted in the
	// denominator it would leave the bar short of its own total for ever,
	// which reads as an import that stalled.
	it('counts only what is still in play, never the resources left behind', () => {
		expect(inPlay(counts({ selected: 3, imported: 2, skipped: 40 }))).toBe(5);
	});

	// A resource that was read and then failed was still read: the bar counts
	// the reading, not the outcome.
	it('counts everything past the read as read', () => {
		expect(readSoFar(counts({ read: 1, matched: 2, review: 3, imported: 4, failed: 5 }))).toBe(
			15
		);
	});

	it('says how far it has got, who is waiting and what landed', () => {
		const line = progressLine(counts({ selected: 28, read: 0, review: 3, imported: 9 }), 40);
		expect(line).toBe('12 of 40 read · 3 need you · 9 imported');
	});

	// "0 need you" sends a seller looking for a question nobody asked.
	it('drops the figures that are zero rather than claiming about nothing', () => {
		expect(progressLine(counts({ selected: 38, read: 2 }), 40)).toBe('2 of 40 read');
	});
});

describe('countsLine', () => {
	it('says what a settled run did', () => {
		expect(countsLine(counts({ imported: 6, skipped: 2, failed: 1 }), 9)).toBe(
			'9 found · 6 imported · 2 left out · 1 with problems'
		);
	});

	it('says a shop is being read rather than inventing a total', () => {
		expect(countsLine(counts(), null)).toBe('Reading what is in your shop.');
	});
});

describe('where a run is read', () => {
	// A spreadsheet run's review belongs on the batch page, which holds the
	// report the rows came from; a separate screen would show the question
	// without the rows it is about.
	it('sends a spreadsheet run to its batch and a shop run to its own page', () => {
		expect(runHref(head({ kind: 'spreadsheet', source: null, batch_id: 'b-2' }))).toBe(
			'/imports/b-2'
		);
		expect(runHref(head({ id: 'r-9' }))).toBe('/imports/runs/r-9');
	});

	it('names a shop run by its shop', () => {
		expect(runName(head({ source: 'Tes' }))).toContain('TES');
		expect(runName(head({ source: null }))).toBe('Spreadsheet');
	});

	// The auto-label is what makes this link exact: a marketplace run labels
	// every resource it creates with the shop it came from, so the summary
	// hands the seller what it made rather than the whole catalogue.
	it('points the finished run at the resources it labelled', () => {
		expect(importedHref('Tes')).toBe('/resources?label=Tes');
		expect(importedHref('Tpt')).toBe('/resources?label=TPT');
		expect(importedHref(null)).toBe('/resources');
	});
});

describe('itemRows', () => {
	it('names a resource by its title, and by its locator where none was read', () => {
		const [named, bare] = itemRows(
			view([item(), item({ locator: 'https://shop/2', ordinal: 2, title: null })])
		);
		expect(named?.name).toBe('Fractions pack');
		expect(bare?.name).toBe('https://shop/2');
	});

	it('renders a price the marketplace gave and an em dash for one it did not', () => {
		const [priced, bare] = itemRows(
			view([item(), item({ locator: 'b', ordinal: 2, price: null })])
		);
		expect(priced?.price).toBe('£4.50');
		expect(bare?.price).toBeNull();
	});

	it('carries the reason a resource was left out or went wrong', () => {
		const rows = itemRows(
			view([
				item({ state: 'skipped', skip_reason: 'not chosen' }),
				item({
					locator: 'b',
					ordinal: 2,
					state: 'failed',
					failure_detail: 'the shop timed out'
				})
			])
		);
		expect(rows[0]?.reason).toBe('not chosen');
		expect(rows[1]?.reason).toBe('the shop timed out');
	});

	it('offers only what has not been answered for', () => {
		const rows = selectionRows(
			view([
				item(),
				item({
					locator: 'b',
					ordinal: 2,
					state: 'imported',
					product_id: 'p-2'
				})
			])
		);
		expect(rows.map((row) => row.locator)).toEqual(['https://shop/1']);
	});
});

describe('selectionBlocked', () => {
	it('says what to do rather than what is wrong', () => {
		expect(selectionBlocked(0, false)).toContain('Tick at least one');
		expect(selectionBlocked(1, false)).toBeNull();
	});

	it('states the send in flight, so a second press has a stated reason', () => {
		expect(selectionBlocked(3, true)).toBe('Your choice is being sent.');
	});
});

describe('reviewCard', () => {
	it('carries the server’s own sentence and never a score', () => {
		const card = reviewCard(pair());
		expect(card.sentence).toBe('The same file, byte for byte: worksheet-pack.pdf, 2.4 MB.');
		expect(card.sentence).not.toMatch(/\d+(\.\d+)?%|score|odds/i);
	});

	// Keeping the side that exists and keeping the side that is only a read
	// are different acts: one creates nothing and one creates a resource, and
	// the card has to say which is which before the seller chooses.
	it('says which side is already a resource and which was only just read', () => {
		const [lo, hi] = reviewCard(pair()).sides;
		expect(lo.standing).toBe('Already in Resources');
		expect(hi.standing).toBe('Just read from your shop');
		expect(hi.product).toBeNull();
	});

	it('addresses the pair by the handles the view gave it', () => {
		const card = reviewCard(pair({ product_lo: 'x', product_hi: 'y' }));
		expect([card.lo, card.hi]).toEqual(['x', 'y']);
		expect(card.sides.map((one) => one.side)).toEqual(['lo', 'hi']);
	});

	// A marketplace that told us nothing has not told us a resource is free.
	it('reads a missing price as unknown rather than as free', () => {
		const [lo] = reviewCard(pair({ lo: side({ price: null }) })).sides;
		expect(lo.price).toBe('—');
		expect(reviewCard(pair()).sides[0].price).toBe('£4.50');
	});

	it('renders the grades as the seller wrote them', () => {
		expect(reviewCard(pair()).sides[0].grades).toBe('Year 3, Year 4');
		expect(reviewCard(pair({ lo: side({ grades: [] }) })).sides[0].grades).toBe('');
	});
});

describe('the settled summary', () => {
	it('states a zero rather than staying silent', () => {
		expect(settledLine(counts())).toBe('0 resources are in Resources.');
	});

	it('counts one resource in the singular', () => {
		expect(settledLine(counts({ imported: 1 }))).toBe('1 resource is in Resources.');
	});

	it('names what did not arrive beside what did', () => {
		expect(settledLine(counts({ imported: 4, skipped: 2, failed: 1 }))).toBe(
			'4 resources are in Resources, 2 left out and 1 did not import.'
		);
	});

	// "Nothing has arrived yet" is true only while something may still
	// arrive; on a settled run it contradicts the stage directly above it.
	it('says why the list is empty differently once the run has settled', () => {
		expect(emptyItemsLine('reading')).toBe('Nothing has arrived yet.');
		expect(emptyItemsLine('done')).toBe('Your shop had nothing in it to bring across.');
		expect(emptyItemsLine('failed')).toContain('before this import stopped');
	});
});
