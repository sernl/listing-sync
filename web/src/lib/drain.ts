// The reconciliation-drain series: one ImportDrainMeasured event per import
// run, narrowed off the ledger and turned into the share the M1 kill gate
// compares across the first ten migrations. Pure, so it tests without a
// component and without a stream.

import type { InventoryId } from '$lib/generated/vocab';

export interface DrainMeasurement {
	source: InventoryId;
	target: InventoryId;
	rows: number;
	terms_seen: number;
	terms_unmapped: number;
	terms_covered: number;
	items_new: number;
	items_already_open: number;
}

export interface DrainRun extends DrainMeasurement {
	seq: number;
	/** Canonical terms the run had to translate; the share's denominator. */
	terms: number;
	/** null rather than zero when the run projected no canonical term at all:
	 *  no terms is no measurement, and zero would read as perfectly drained. */
	share: number | null;
}

const COUNTS = [
	'rows',
	'terms_seen',
	'terms_unmapped',
	'terms_covered',
	'items_new',
	'items_already_open'
] as const;

/** Narrows one stream payload, or null for anything that is not a drain
 *  measurement this client understands. */
export function readMeasurement(payload: unknown): DrainMeasurement | null {
	if (typeof payload !== 'object' || payload === null) {
		return null;
	}
	const body = payload as Record<string, unknown>;
	if (typeof body.source !== 'string' || typeof body.target !== 'string') {
		return null;
	}
	for (const key of COUNTS) {
		if (typeof body[key] !== 'number' || !Number.isFinite(body[key])) {
			return null;
		}
	}
	return body as unknown as DrainMeasurement;
}

export function toRun(seq: number, measurement: DrainMeasurement): DrainRun {
	const terms =
		measurement.terms_covered + measurement.items_new + measurement.items_already_open;
	return {
		...measurement,
		seq,
		terms,
		share: terms === 0 ? null : measurement.items_new / terms
	};
}

export interface GateWindow {
	first: DrainRun | null;
	tenth: DrainRun | null;
	/** first.share - tenth.share; positive means the queue is draining.
	 *  null until ten runs with a measurable share exist. */
	fall: number | null;
}

/** The kill gate reads the first ten migrations: the share of canonical terms
 *  raising a new item must fall materially between the first and the tenth. */
export function gateWindow(runs: DrainRun[]): GateWindow {
	const first = runs[0] ?? null;
	const tenth = runs.length >= 10 ? (runs[9] ?? null) : null;
	const fall =
		first?.share != null && tenth?.share != null ? first.share - tenth.share : null;
	return { first, tenth, fall };
}

export function formatShare(share: number | null): string {
	return share === null ? '—' : `${(share * 100).toFixed(1)}%`;
}
