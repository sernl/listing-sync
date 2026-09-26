import { describe, expect, it } from 'vitest';
import {
	deviceConditionIsCurrent,
	emptyItemsLine,
	importedHref,
	itemRows,
	pageCount,
	pageSummary,
	reasonLine,
	reviewCard,
	runBadge,
	runHref,
	runName,
	selectionBlocked,
	selectionRows,
	settledLine,
	stageFrom
} from './run-view';
import type {
	ImportRunCounts,
	ImportRunHead,
	ImportRunItemView,
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
		retry_of: null,
		execution: {
			owner_device: null, attempt: 0, lease_expires_at: null,
			last_contact_at: null, last_progress_at: null, stage: 'waiting',
			reason_code: null, reason: null, discovered: 0, processed: 0, described: 0,
			enumeration_complete: false, selected_total: null, commit_authorised: false
		},
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
	it('does not present an interrupted reader as active work', () => {
		const run = head();
		run.execution.stage = 'interrupted';
		run.execution.reason_code = 'lease_expired';
		expect(stageFrom(run)).toBe('interrupted');
	});

	it('does not present description completion as authorised catalogue addition', () => {
		const run = head({ state: 'committing' });
		run.execution.stage = 'committing';
		expect(stageFrom(run)).toBe('confirming');
	});

});

describe('runBadge', () => {
	it('does not congratulate a completed run that left resources out', () => {
		const run = head({ state: 'complete', counts: counts({ imported: 3, failed: 1 }) });
		run.execution.stage = 'completed';
		expect(runBadge(run).tone).toBe('warn');
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
		const [named, bare] = itemRows([
			item(),
			item({ locator: 'https://shop/2', ordinal: 2, title: null })
		]);
		expect(named?.name).toBe('Fractions pack');
		expect(bare?.name).toBe('https://shop/2');
	});

	it('renders a price the marketplace gave and an em dash for one it did not', () => {
		const [priced, bare] = itemRows([item(), item({ locator: 'b', ordinal: 2, price: null })]);
		expect(priced?.price).toBe('£4.50');
		expect(bare?.price).toBeNull();
	});

	it('carries the reason a resource was left out or went wrong', () => {
		const rows = itemRows([
			item({ state: 'skipped', skip_reason: 'not chosen' }),
			item({
				locator: 'b',
				ordinal: 2,
				state: 'failed',
				failure_detail: 'the shop timed out'
			})
		]);
		expect(rows[0]?.reason).toBe('not chosen');
		expect(rows[1]?.reason).toBe('the shop timed out');
	});

	it('offers only what has not been answered for', () => {
		const rows = selectionRows([
			item(),
			item({
				locator: 'b',
				ordinal: 2,
				state: 'imported',
				product_id: 'p-2'
			})
		]);
		expect(rows.map((row) => row.locator)).toEqual(['https://shop/1']);
	});
});

describe('selectionBlocked', () => {
	it('says what to do rather than what is wrong', () => {
		expect(selectionBlocked(0, false)).toContain('Tick at least one');
		expect(selectionBlocked(1, false)).toBeNull();
	});

	it('states the send in flight, so a second press has a stated reason', () => {
		expect(selectionBlocked(3, true)).toBe('Sending your choice.');
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
		expect(hi.standing).toBe('New from your shop');
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
		expect(emptyItemsLine('done')).toBe('Your shop had nothing to import.');
		expect(emptyItemsLine('failed')).toContain('before this import stopped');
	});
});

describe('why an import stopped', () => {
	// The codes are transport diagnostics. A seller shown "lease_expired"
	// learns nothing they can do, which is the whole reason this table exists.
	it('says what the seller does next, never the code', () => {
		const lease = reasonLine('lease_expired', null);
		expect(lease).toContain('Resume');
		expect(lease).not.toContain('lease');
		expect(reasonLine('missing_session', null)).toContain('Sign in');
		expect(reasonLine('client_update_required', null)).toContain('Update');
	});

	// The server's own sentence is not thrown away: where there is no code to
	// read, it is the only account of what happened.
	it('keeps the server’s own words where it sent no code', () => {
		expect(reasonLine(null, 'The shop answered nothing for ten minutes.')).toBe(
			'The shop answered nothing for ten minutes.'
		);
		expect(reasonLine(null, null)).toBeNull();
	});

	// The reported defect: a lease that expired once and was recovered from
	// stays on the run, and the review screen read it as a live failure.
	it('reads the state, not the reason, to decide whether a failure is live', () => {
		const expired = { reason_code: 'lease_expired' as const, reason: null };
		const reviewing = head({ state: 'reviewing', execution: { ...head().execution, ...expired } });
		expect(deviceConditionIsCurrent(reviewing)).toBe(false);
		const waiting = head({ state: 'reading', execution: { ...head().execution, ...expired } });
		expect(deviceConditionIsCurrent(waiting)).toBe(true);
	});
});

describe('the pager', () => {
	// A list with nothing in it is still on page one: zero pages would leave
	// the pager reading "Page 1 of 0".
	it('counts at least one page, and one more for a partial page', () => {
		expect(pageCount(0, 25)).toBe(1);
		expect(pageCount(25, 25)).toBe(1);
		expect(pageCount(26, 25)).toBe(2);
	});

	it('states the slice against the whole filtered list, not the page', () => {
		expect(pageSummary(25, 1, 26, 'resources')).toBe('26–26 of 26 resources');
		expect(pageSummary(0, 10, 143, 'imports')).toBe('1–10 of 143 imports');
		expect(pageSummary(0, 0, 0, 'imports')).toBe('No imports');
	});
});
