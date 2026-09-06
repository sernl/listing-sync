import { describe, expect, it } from 'vitest';
import {
	emptyState,
	formatPoints,
	formatShare,
	gateWindow,
	readMeasurement,
	seriesByOrg,
	toRun,
	type DrainMeasurement,
	type DrainRun
} from './drain';

const measurement: DrainMeasurement = {
	source: 'TesGb',
	target: 'TesNz',
	rows: 1,
	terms_seen: 3,
	terms_unmapped: 1,
	terms_covered: 1,
	items_new: 1,
	items_already_open: 0
};

function runWithShare(seq: number, covered: number, fresh: number): DrainRun {
	return toRun(seq, { ...measurement, terms_covered: covered, items_new: fresh });
}

describe('the drain series', () => {
	it('takes the share over canonical terms, already-open ones included', () => {
		const run = toRun(7, { ...measurement, terms_covered: 2, items_new: 1, items_already_open: 1 });
		expect(run.terms).toBe(4);
		expect(run.share).toBe(0.25);
		expect(run.seq).toBe(7);
	});

	it('reports no share rather than a drained-looking zero when nothing projected', () => {
		const run = toRun(1, {
			...measurement,
			terms_covered: 0,
			items_new: 0,
			items_already_open: 0
		});
		expect(run.terms).toBe(0);
		expect(run.share).toBeNull();
		expect(formatShare(run.share)).toBe('—');
	});

	it('keeps unmapped terms out of the share so an ingest gap cannot fake a drain', () => {
		const leaky = toRun(1, { ...measurement, terms_unmapped: 99 });
		const clean = toRun(1, { ...measurement, terms_unmapped: 0 });
		expect(leaky.share).toBe(clean.share);
		expect(leaky.terms_unmapped).toBe(99);
	});

	it('rejects a payload that is not a drain measurement', () => {
		expect(readMeasurement(null)).toBeNull();
		expect(readMeasurement({ items: 2 })).toBeNull();
		expect(readMeasurement({ ...measurement, items_new: 'two' })).toBeNull();
		expect(readMeasurement(measurement)).toEqual(measurement);
	});

	// A row from a build that knows an inventory this one does not: `source` is
	// a string, so the old check admitted it, and the row was then typed
	// `InventoryId` while being none of them. Every total map keyed on that
	// union answers `undefined` for it, so the direction cell drew blank and
	// the refused-row count -- the page's whole account of what it dropped --
	// never saw the row at all.
	it('refuses a payload naming an inventory this bundle does not know', () => {
		expect(readMeasurement({ ...measurement, source: 'TesIe' })).toBeNull();
		expect(readMeasurement({ ...measurement, target: 'TesIe' })).toBeNull();
		const { series, unreadable } = seriesByOrg([
			{ org: 'a', org_name: 'Alpha', org_seq: 1, payload: { ...measurement, source: 'TesIe' } },
			{ org: 'a', org_name: 'Alpha', org_seq: 2, payload: measurement }
		]);
		expect(unreadable).toBe(1);
		expect(series[0]?.runs.map((run) => run.seq)).toEqual([2]);
	});

	// A difference of two percentages is points, not a percentage. The gate's
	// own sentence is where the distinction is read and acted on, so it is
	// asserted as the rendered string rather than as the number behind it.
	it('renders the difference of two shares in points rather than percent', () => {
		expect(formatPoints(0.125 - 0.04)).toBe('8.5 percentage points');
		expect(formatShare(0.125 - 0.04)).toBe('8.5%');
		expect(formatPoints(null)).toBe('—');
	});

	it('holds the gate open until a tenth run exists', () => {
		const nine = Array.from({ length: 9 }, (_, index) => runWithShare(index + 1, 1, 1));
		expect(gateWindow(nine).tenth).toBeNull();
		expect(gateWindow(nine).fall).toBeNull();
		expect(gateWindow(nine).gap).toBe('short');
		expect(gateWindow([]).first).toBeNull();
		expect(gateWindow([]).gap).toBe('short');
	});

	// Twelve runs is not a short window, and saying "12 of 10 migrations
	// recorded" is false rather than merely vague. The window closed; one of
	// its ends has no share, which is a different thing to report.
	it('names a closed window with no measurable end rather than counting it short', () => {
		const runs = [
			toRun(1, { ...measurement, terms_covered: 0, items_new: 0, items_already_open: 0 }),
			...Array.from({ length: 11 }, (_, index) => runWithShare(index + 2, 2, 2))
		];
		const window = gateWindow(runs);
		expect(runs).toHaveLength(12);
		expect(window.first?.share).toBeNull();
		expect(window.tenth).not.toBeNull();
		expect(window.fall).toBeNull();
		expect(window.gap).toBe('unmeasurable');
	});

	// The one invariant the page's three-way branch rests on: a fall and a gap
	// are never both present and never both absent, so no reachable window
	// leaves the page with no sentence to render or two.
	it('reports a fall or a reason, never both and never neither', () => {
		const windows = [
			gateWindow([]),
			gateWindow(Array.from({ length: 9 }, (_, index) => runWithShare(index + 1, 1, 1))),
			gateWindow([
				toRun(1, { ...measurement, terms_covered: 0, items_new: 0, items_already_open: 0 }),
				...Array.from({ length: 11 }, (_, index) => runWithShare(index + 2, 2, 2))
			]),
			gateWindow([
				runWithShare(1, 0, 4),
				...Array.from({ length: 8 }, (_, index) => runWithShare(index + 2, 2, 2)),
				runWithShare(10, 3, 1)
			])
		];
		for (const window of windows) {
			expect(window.fall === null).toBe(window.gap !== null);
		}
	});

	it('measures the fall between the first catalogue and the tenth', () => {
		const runs = [
			runWithShare(1, 0, 4),
			...Array.from({ length: 8 }, (_, index) => runWithShare(index + 2, 2, 2)),
			runWithShare(10, 3, 1)
		];
		expect(gateWindow(runs).first?.share).toBe(1);
		expect(gateWindow(runs).tenth?.share).toBe(0.25);
		expect(gateWindow(runs).fall).toBe(0.75);
		expect(gateWindow(runs).gap).toBeNull();
	});
});

describe('the operator series', () => {
	function row(org: string, name: string, seq: number, extra: Partial<DrainMeasurement> = {}) {
		return { org, org_name: name, org_seq: seq, payload: { ...measurement, ...extra } };
	}

	it('keeps each tenant a series of its own, ordered by ledger position', () => {
		const { series, unreadable } = seriesByOrg([
			row('b', 'Beta', 5),
			row('a', 'Alpha', 9),
			row('b', 'Beta', 2),
			row('a', 'Alpha', 3)
		]);
		expect(series.map((entry) => entry.org_name)).toEqual(['Alpha', 'Beta']);
		expect(series[0]?.runs.map((run) => run.seq)).toEqual([3, 9]);
		expect(series[1]?.runs.map((run) => run.seq)).toEqual([2, 5]);
		expect(unreadable).toBe(0);
	});

	// The reason the grouping happens before the window is taken. Pooled, these
	// ten rows present a first at 100%, a tenth at 25% and a confident fall of
	// 75% -- a number belonging to no tenant, since the first and the tenth are
	// different organisations. Split, neither has reached ten migrations and
	// both correctly report no fall at all.
	it('never measures a fall between one tenant and another', () => {
		const rows = [
			...Array.from({ length: 5 }, (_, index) =>
				row('a', 'Alpha', index + 1, { terms_covered: 0, items_new: 4 })
			),
			...Array.from({ length: 5 }, (_, index) =>
				row('b', 'Beta', index + 1, { terms_covered: 3, items_new: 1 })
			)
		];
		const pooled = gateWindow(
			rows.map((entry, index) => toRun(index + 1, entry.payload as DrainMeasurement))
		);
		expect(pooled.fall).toBe(0.75);

		const { series } = seriesByOrg(rows);
		expect(series).toHaveLength(2);
		for (const entry of series) {
			expect(entry.runs).toHaveLength(5);
			expect(entry.gate.tenth).toBeNull();
			expect(entry.gate.fall).toBeNull();
		}
	});

	it('drops a row whose payload is not a measurement and keeps the rest', () => {
		const { series, unreadable } = seriesByOrg([
			row('a', 'Alpha', 1),
			{ org: 'a', org_name: 'Alpha', org_seq: 2, payload: { nonsense: true } },
			row('a', 'Alpha', 3)
		]);
		expect(series).toHaveLength(1);
		expect(series[0]?.runs.map((run) => run.seq)).toEqual([1, 3]);
		expect(unreadable).toBe(1);
	});

	// The count is what keeps the drop from being silent, so it is asserted
	// where the page would otherwise show an ordinary empty state: every row
	// refused reads exactly like no row recorded once the rows are gone.
	it('counts every refused row, including when none survives', () => {
		const malformed = (seq: number) => ({
			org: 'a',
			org_name: 'Alpha',
			org_seq: seq,
			payload: { nonsense: true }
		});
		const { series, unreadable } = seriesByOrg([malformed(1), malformed(2)]);
		expect(series).toEqual([]);
		expect(unreadable).toBe(2);
		expect(emptyState(unreadable).headline).toBe('No drain report could be read');
	});

	// The empty page has two causes and one shape, so this is the assertion
	// that stops a fault being announced in the voice of a quiet state.
	it('separates nothing recorded from everything refused in the empty state', () => {
		expect(emptyState(0).headline).toBe('No import has recorded a drain report yet');
		expect(emptyState(0).icon).toBe('circle-check');
		expect(emptyState(1).headline).not.toBe(emptyState(0).headline);
		expect(emptyState(1).body).not.toBe(emptyState(0).body);
		expect(emptyState(1).icon).toBe('triangle-alert');
	});

	it('reports no tenant at all rather than an empty series when nothing is measured', () => {
		expect(seriesByOrg([])).toEqual({ series: [], unreadable: 0 });
	});
});
