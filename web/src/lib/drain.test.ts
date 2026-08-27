import { describe, expect, it } from 'vitest';
import {
	formatShare,
	gateWindow,
	readMeasurement,
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

	it('holds the gate open until a tenth run exists', () => {
		const nine = Array.from({ length: 9 }, (_, index) => runWithShare(index + 1, 1, 1));
		expect(gateWindow(nine).tenth).toBeNull();
		expect(gateWindow(nine).fall).toBeNull();
		expect(gateWindow([]).first).toBeNull();
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
	});
});
