// What the batch page's model must not get wrong: a file placed on a row
// nobody named, a state added in Rust blanking the page, a row addressed by
// its index rather than by the number in the seller's own margin, a count
// invented where nothing was read, and an unread list rendering as an empty
// one.

import { describe, expect, it } from 'vitest';
import type { ImportBatchView, ImportRowView, RowStateView } from '$lib/api';
import { readState } from '$lib/pages/automations/read-state';
import {
	BATCH_STATES,
	NO_ROW_NAMED_IT,
	ROW_ALREADY_TAKEN,
	ROW_STATES,
	SEVERAL_ROWS_NAMED_IT,
	awaitingFrom,
	awaitingRows,
	awaitingSay,
	batchClosedSay,
	batchRows,
	listCopy,
	matchFiles,
	previewOf,
	previewSentence,
	progressFrom,
	progressLine,
	reportRows,
	resourceHref,
	stageFrom,
	stemOf,
	tallyRows,
	headroomLine,
	presentStage
} from './sheet-view';

function row(over: Partial<ImportRowView> = {}): ImportRowView {
	return {
		sheet: 'TES',
		ordinal: 2,
		inventory: 'Tes',
		intent: 'draft',
		state: 'parsed',
		problems: [],
		file_name: 'fractions.pdf',
		file_attached: false,
		failure_detail: null,
		product: null,
		...over
	};
}

function batch(over: Partial<ImportBatchView> = {}): ImportBatchView {
	return {
		id: 'b1',
		source_name: 'my-listings.xlsx',
		state: 'parsed',
		row_count: 3,
		live_count: 1,
		failed_count: 0,
		created_at: 1_700_000_000_000,
		expires_at: 1_700_600_000_000,
		settled_at: null,
		failure_detail: null,
		...over
	};
}

describe('placing dropped files on the rows that named them', () => {
	it('places a file whose name a row wrote exactly', () => {
		const placed = matchFiles(
			['fractions.pdf'],
			[{ sheet: 'TES', ordinal: 4, fileName: 'fractions.pdf' }]
		);
		expect(placed).toEqual([
			{
				name: 'fractions.pdf',
				kind: 'matched',
				row: { sheet: 'TES', ordinal: 4 },
				pass: 'exact'
			}
		]);
	});

	it('places a file whose name differs only in case', () => {
		const placed = matchFiles(
			['Fractions.PDF'],
			[{ sheet: 'TES', ordinal: 4, fileName: 'fractions.pdf' }]
		);
		expect(placed[0]).toMatchObject({ kind: 'matched', pass: 'case' });
	});

	it('places a file whose name matches once the extension is dropped', () => {
		const placed = matchFiles(
			['fractions.zip'],
			[{ sheet: 'TES', ordinal: 4, fileName: 'fractions.pdf' }]
		);
		expect(placed[0]).toMatchObject({ kind: 'matched', pass: 'stem' });
	});

	it('never guesses when more than one row named the file', () => {
		const placed = matchFiles(
			['fractions.pdf'],
			[
				{ sheet: 'TES', ordinal: 4, fileName: 'fractions.pdf' },
				{ sheet: 'TPT', ordinal: 9, fileName: 'fractions.pdf' }
			]
		);
		expect(placed).toEqual([
			{ name: 'fractions.pdf', kind: 'unplaced', reason: SEVERAL_ROWS_NAMED_IT }
		]);
	});

	it('takes the tightest pass answer, though a looser pass would be ambiguous', () => {
		// Two rows name the same stem and one names the file exactly. The exact
		// pass answers first and answers one, so the ambiguity below it never
		// decides anything.
		const placed = matchFiles(
			['fractions.pdf'],
			[
				{ sheet: 'TES', ordinal: 4, fileName: 'fractions.pdf' },
				{ sheet: 'TPT', ordinal: 9, fileName: 'fractions.zip' }
			]
		);
		expect(placed[0]).toMatchObject({ kind: 'matched', pass: 'exact' });
	});

	it('sends a file no row named to the strip', () => {
		const placed = matchFiles(
			['stray.pdf'],
			[{ sheet: 'TES', ordinal: 4, fileName: 'fractions.pdf' }]
		);
		expect(placed).toEqual([{ name: 'stray.pdf', kind: 'unplaced', reason: NO_ROW_NAMED_IT }]);
	});

	it('never binds two files of one drop to one row', () => {
		const placed = matchFiles(
			['fractions.pdf', 'FRACTIONS.pdf'],
			[{ sheet: 'TES', ordinal: 4, fileName: 'fractions.pdf' }]
		);
		expect(placed[0]).toMatchObject({ kind: 'matched' });
		expect(placed[1]).toEqual({
			name: 'FRACTIONS.pdf',
			kind: 'unplaced',
			reason: ROW_ALREADY_TAKEN
		});
	});

	it('leaves a row that declared no filename to be placed by hand', () => {
		const placed = matchFiles(['fractions.pdf'], [{ sheet: 'TPT', ordinal: 3, fileName: null }]);
		expect(placed[0]).toMatchObject({ kind: 'unplaced', reason: NO_ROW_NAMED_IT });
	});

	it('keeps a name that is all extension whole, so two of them do not collide', () => {
		expect(stemOf('.keep')).toBe('.keep');
		const placed = matchFiles(
			['.keep'],
			[
				{ sheet: 'TES', ordinal: 4, fileName: '.gitignore' },
				{ sheet: 'TPT', ordinal: 9, fileName: '.npmrc' }
			]
		);
		expect(placed[0]).toMatchObject({ kind: 'unplaced', reason: NO_ROW_NAMED_IT });
	});
});

describe('the stage a batch is in', () => {
	it('recognises every state the wire declares', () => {
		for (const state of BATCH_STATES) {
			const stage = stageFrom(state, null, [row()]);
			expect(stage.kind, state).not.toBe('unrecognised');
			expect(presentStage(stage).label.length, state).toBeGreaterThan(0);
		}
		// The sweep is worth taking only if it covers the whole vocabulary.
		expect(BATCH_STATES).toHaveLength(6);
	});

	it('degrades a state it has never heard of to saying so', () => {
		const stage = stageFrom('reticulating', null, [row()]);
		expect(stage).toEqual({ kind: 'unrecognised', state: 'reticulating' });
		expect(presentStage(stage).line).toContain('reticulating');
	});

	it("carries the server's own words when a batch finished with problems", () => {
		const stage = stageFrom('failed', '2 rows did not import', [row({ state: 'failed' })]);
		expect(presentStage(stage).line).toBe('2 rows did not import');
	});

	it('says nothing reached a marketplace once the import has finished', () => {
		const line = presentStage(stageFrom('imported', null, [row({ state: 'created' })])).line;
		expect(line).toContain('Nothing has been sent to a marketplace');
	});
	// The matcher runs inside the commit, so a batch that is importing with a
	// pair open is a commit that has stopped and is waiting for an answer.
	// Rendering it as "creating your resources" would claim work that is not
	// happening, and it is what leaves a seller watching a bar that never
	// moves.
	it('reads a parked pair as a question rather than as a commit in progress', () => {
		const stage = stageFrom('importing', null, [row({ state: 'attached' })], 2);
		expect(stage.kind).toBe('review');
		const shown = presentStage(stage);
		expect(shown.label).toBe('Needs you');
		expect(shown.tone).toBe('warn');
		expect(shown.line).toContain('2 of these look');
	});

	// A pair parked before the commit is the same question: the spreadsheet
	// commit creates the run, so the pairs can be open at any of the three
	// pre-settled states.
	it('asks the question at every state a commit can be paused in', () => {
		for (const state of ['parsed', 'attaching', 'importing']) {
			expect(stageFrom(state, null, [row()], 1).kind, state).toBe('review');
		}
	});

	it('counts one pair in the singular', () => {
		expect(presentStage(stageFrom('importing', null, [row()], 1)).line).toContain(
			'1 of these looks'
		);
	});

	// A settled batch is not asking anything, whatever is still parked: a
	// parked pair never blocks an import, and a finished one has nothing left
	// to block.
	it('does not reopen a settled batch to ask about a pair', () => {
		expect(stageFrom('imported', null, [row({ state: 'created' })], 3).kind).toBe('imported');
	});

	// The commit is the only thing a parked pair holds up, so the stage the
	// page renders without one must be unchanged: nothing about the ordinary
	// path moves when the matcher finds nothing.
	it('leaves every stage alone when nothing is parked', () => {
		for (const state of BATCH_STATES) {
			expect(stageFrom(state, null, [row()], 0)).toEqual(stageFrom(state, null, [row()]));
		}
	});

	it('counts every row state under exactly one figure', () => {
		const rows = ROW_STATES.map((state: RowStateView) => row({ state }));
		const tally = tallyRows(rows);
		const parts =
			tally.waiting +
			tally.attached +
			tally.creating +
			tally.created +
			tally.refused +
			tally.skipped +
			tally.unrecognised;
		expect(parts).toBe(tally.total);
		expect(tally.total).toBe(ROW_STATES.length);
	});

	it('counts a row state it does not know rather than dropping it', () => {
		const tally = tallyRows([row({ state: 'levitating' as RowStateView })]);
		expect(tally.unrecognised).toBe(1);
		expect(tally.total).toBe(1);
	});
});

describe('the report addressing rows', () => {
	it("cites the seller's own spreadsheet row number rather than an index", () => {
		const rendered = reportRows([row({ ordinal: 7 }), row({ ordinal: 12 })]);
		expect(rendered[0].ordinal).toBe(7);
		expect(rendered[0].label).toBe('TES, row 7');
		expect(rendered[1].label).toBe('TES, row 12');
		// The index is 0 and 1; neither may appear as the row's number.
		expect(rendered.map((entry) => entry.label)).not.toContain('TES, row 0');
		expect(rendered.map((entry) => entry.label)).not.toContain('TES, row 1');
	});

	it('keys rows by sheet and row number, so two tabs may share a number', () => {
		const rendered = reportRows([row({ ordinal: 4 }), row({ sheet: 'TPT', ordinal: 4 })]);
		expect(new Set(rendered.map((entry) => entry.key)).size).toBe(2);
	});

	it('does not name the marketplace a second time when the tab already did', () => {
		expect(reportRows([row({ sheet: 'TES', inventory: 'Tes' })])[0].marketplace).toBeNull();
		expect(reportRows([row({ sheet: 'Sheet1', inventory: 'Tes' })])[0].marketplace).toBe(
			'TES'
		);
		expect(reportRows([row({ sheet: 'Teachouse', inventory: null })])[0].marketplace).toBeNull();
	});

	it('names a row by its title only when the wire carried one', () => {
		expect(reportRows([row({ title: 'Fractions pack' })])[0].name).toBe('Fractions pack');
		expect(reportRows([row({ file_name: 'f.pdf' })])[0].name).toBe('f.pdf');
		expect(reportRows([row({ file_name: null })])[0].name).toBeNull();
	});

	it('offers no resource link for a row that names no resource', () => {
		expect(resourceHref(row({ state: 'created' }))).toBeNull();
		expect(resourceHref(row({ state: 'created', product: 'p-1' }))).toBe('/resources/p-1');
	});

	it('offers no resource link for a row the server sent a null product for', () => {
		expect(resourceHref(row({ state: 'parsed', product: null }))).toBeNull();
		expect(resourceHref(row({ state: 'created', product: null }))).toBeNull();
	});

	it('offers the link only from the states a resource exists in', () => {
		// The identifier arrives before the resource does: a commit reserves it
		// when it claims the row, so `creating` names one nothing answers to.
		expect(resourceHref(row({ state: 'created', product: 'p-1' }))).toBe('/resources/p-1');
		expect(resourceHref(row({ state: 'published', product: 'p-1' }))).toBe('/resources/p-1');
		const unlinked: RowStateView[] = ['parsed', 'attached', 'creating', 'failed', 'skipped'];
		for (const state of unlinked) {
			expect(resourceHref(row({ state, product: 'p-1' })), state).toBeNull();
		}
	});

	it('renders no Open control for a row still being created', () => {
		expect(reportRows([row({ state: 'creating', product: 'p-1' })])[0].href).toBeNull();
		expect(reportRows([row({ state: 'created', product: 'p-1' })])[0].href).toBe('/resources/p-1');
	});
});

describe('what a commit would do', () => {
	it('counts what will be created and what will be left alone', () => {
		const preview = previewOf(
			tallyRows([
				row({ state: 'parsed' }),
				row({ state: 'attached' }),
				row({ state: 'failed' }),
				row({ state: 'created' })
			])
		);
		expect(preview).toEqual({ create: 2, leave: 1, done: 1, unknown: 0 });
		expect(previewSentence(preview)).toBe(
			'2 resources will be created in your catalogue. 1 row was refused when the sheet was read, and will be left alone.'
		);
	});

	it('drops the refusal clause rather than claiming zero of them', () => {
		const preview = previewOf(tallyRows([row(), row()]));
		expect(previewSentence(preview)).toBe('2 resources will be created in your catalogue.');
	});

	it('keeps a row in an unknown state out of both figures', () => {
		const preview = previewOf(tallyRows([row({ state: 'hovering' as RowStateView })]));
		expect(preview).toEqual({ create: 0, leave: 0, done: 0, unknown: 1 });
	});

	it('holds the gate until every marketplace row has its bytes', () => {
		const rows = [
			row({ inventory: 'Tes', file_attached: false }),
			row({ inventory: null, sheet: 'Teachouse', file_attached: false }),
			row({ state: 'failed', file_attached: false })
		];
		// The commit route's gate, stated the way the page states it: the rows
		// still awaiting bytes, and none once the marketplace row is dropped.
		expect(awaitingRows(rows)).toHaveLength(1);
		expect(awaitingRows(rows)[0].sheet).toBe('TES');
		expect(awaitingRows([rows[1], rows[2]])).toHaveLength(0);
	});

	it("reads the bar off the chunk's remaining and total, not off its own figures", () => {
		// `applied` and `failed` are this chunk's five rows. Summing them across
		// chunks would keep a second ledger of what the server already counts,
		// and a seller who reloaded the page mid-import would arrive with
		// nothing in it; total less remaining is the same figure either way.
		const chunk = {
			applied: 4,
			skipped: 0,
			failed: 1,
			total: 10,
			remaining: 4,
			complete: false,
			batch_state: 'importing' as const
		};
		expect(progressFrom(null)).toEqual({ kind: 'unstarted' });
		expect(progressFrom(chunk)).toEqual({
			kind: 'running',
			settled: 6,
			total: 10,
			fraction: 0.6
		});
		expect(progressLine(progressFrom(chunk))).toBe('6 of 10 rows done.');
	});

	it('is finished when no row remains, whatever that chunk itself applied', () => {
		const last = {
			applied: 2,
			skipped: 1,
			failed: 0,
			total: 10,
			remaining: 0,
			complete: true,
			batch_state: 'imported' as const
		};
		expect(progressFrom(last)).toEqual({ kind: 'complete', settled: 10, total: 10 });
		expect(progressLine(progressFrom(last))).toBe('All 10 rows done.');
		// A batch the parse refused every row of has nothing to do and is over
		// on its first chunk, rather than dividing by the rows it does not have.
		expect(progressFrom({ ...last, applied: 0, skipped: 0, total: 0 })).toEqual({
			kind: 'complete',
			settled: 0,
			total: 0
		});
	});

	it('states storage only once an upload has reported it', () => {
		expect(headroomLine(null)).toBeNull();
		expect(headroomLine({ stored_bytes: 1024, storage_bytes_max: 2048 })).toBe(
			'1.0 KB of 2.0 KB stored'
		);
	});
});

describe('a list that has not been read', () => {
	it('says so rather than saying there are none', () => {
		const unread = listCopy(readState(false, true, []));
		const empty = listCopy(readState(true, false, []));
		expect(unread).not.toBeNull();
		expect(empty).not.toBeNull();
		expect(unread?.title).not.toBe(empty?.title);
		expect(unread?.title).toContain('could not be read');
		expect(unread?.body).toContain('Nothing has been changed');
		expect(empty?.title).toContain('No spreadsheet import yet');
	});

	it('says nothing at all in place of rows it has', () => {
		expect(listCopy(readState(true, false, [batch()]))).toBeNull();
	});

	it('is still reading before it has either', () => {
		expect(listCopy(readState(false, false, []))?.title).toContain('Reading');
	});

	it('renders a batch row from the counts it read', () => {
		const [only] = batchRows([batch({ row_count: 3, failed_count: 1 })]);
		expect(only.line).toBe('3 rows, 1 refused when it was read');
		expect(only.href).toBe('/imports/b1');
		expect(only.label).toBe('Read');
		expect(only.open).toBe(true);
	});

	it('leaves the refusal clause off a batch with none', () => {
		expect(batchRows([batch({ row_count: 1, failed_count: 0 })])[0].line).toBe('1 row');
	});
});

describe('why a bind was refused 409', () => {
	it('says the import is still running when the refusal names importing', () => {
		const said = batchClosedSay({ batch_state: 'importing' });
		expect(said).toContain('being created right now');
		expect(said).not.toContain('given up');
	});

	it('says the import is over when the refusal names a settled state', () => {
		for (const state of ['imported', 'failed', 'abandoned']) {
			expect(batchClosedSay({ batch_state: state })).toBe(
				'This import has been finished or given up, so no more files can be added.'
			);
		}
	});

	it('says nothing of its own when the refusal carries no state it can read', () => {
		expect(batchClosedSay(undefined)).toBeNull();
		expect(batchClosedSay(null)).toBeNull();
		expect(batchClosedSay({})).toBeNull();
		expect(batchClosedSay({ batch_state: 'nonesuch' })).toBeNull();
		expect(batchClosedSay('importing')).toBeNull();
	});

	it('says nothing of its own for a state that does take files', () => {
		expect(batchClosedSay({ batch_state: 'parsed' })).toBeNull();
		expect(batchClosedSay({ batch_state: 'attaching' })).toBeNull();
	});
});
describe('a commit refused for want of files', () => {
	it('reads the count and the rows off the refusal it came with', () => {
		expect(
			awaitingFrom({
				awaiting: 2,
				rows: [
					{ sheet: 'TES', ordinal: 3 },
					{ sheet: 'TPT', ordinal: 7 }
				]
			})
		).toEqual({
			count: 2,
			rows: [
				{ sheet: 'TES', ordinal: 3 },
				{ sheet: 'TPT', ordinal: 7 }
			]
		});
	});

	it('carries a count that travelled with no rows at all', () => {
		expect(awaitingFrom({ awaiting: 4 })).toEqual({ count: 4, rows: [] });
	});

	it('keeps only the rows the refusal named completely', () => {
		// A row missing its tab or its number cannot be addressed in the terms
		// the seller reads, and one filled in from a default would send them to
		// the wrong margin.
		expect(
			awaitingFrom({
				awaiting: 3,
				rows: [{ sheet: 'TES' }, { ordinal: 3 }, 'TES, row 4', { sheet: 'TPT', ordinal: 7 }]
			})
		).toEqual({ count: 3, rows: [{ sheet: 'TPT', ordinal: 7 }] });
	});

	it('says nothing of its own for a refusal that is not this gate', () => {
		expect(awaitingFrom({ batch_state: 'imported' })).toBeNull();
		expect(awaitingFrom(null)).toBeNull();
		expect(awaitingFrom(undefined)).toBeNull();
		expect(awaitingFrom('4')).toBeNull();
		expect(awaitingFrom({ awaiting: '4' })).toBeNull();
	});

	it('names the rows in the margin numbers the report already uses', () => {
		expect(
			awaitingSay({
				count: 2,
				rows: [
					{ sheet: 'TES', ordinal: 3 },
					{ sheet: 'TPT', ordinal: 7 }
				]
			})
		).toBe('2 rows still need their files before this import can run: TES, row 3; TPT, row 7.');
	});

	it('says how many more there are where the server named only the first', () => {
		expect(awaitingSay({ count: 25, rows: [{ sheet: 'TES', ordinal: 3 }] })).toBe(
			'25 rows still need their files before this import can run: TES, row 3, and 24 more.'
		);
	});

	it('speaks of a single row in the singular', () => {
		expect(awaitingSay({ count: 1, rows: [{ sheet: 'TPT', ordinal: 7 }] })).toBe(
			'One row still needs its file before this import can run: TPT, row 7.'
		);
	});

	it('states the count alone when the refusal named no row', () => {
		expect(awaitingSay({ count: 3, rows: [] })).toBe(
			'3 rows still need their files before this import can run.'
		);
	});
});
