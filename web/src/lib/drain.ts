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

/** One row of the operator route: an organisation, the ledger position the
 *  measurement was written at, and the payload verbatim.
 *
 *  The payload arrives unparsed because it is `jsonb` in the ledger and
 *  nothing constrains its shape at rest. Narrowing it here rather than in the
 *  server keeps one malformed historical row from emptying the whole page:
 *  `readMeasurement` drops that row and the rest of the series still draws. */
export interface DrainRow {
	org: string;
	org_name: string;
	org_seq: number;
	payload: unknown;
}

/** One organisation's series, with its own gate window.
 *
 *  The split by organisation happens before the window is taken, and that
 *  ordering is the whole point rather than a presentation choice. The gate
 *  compares a tenant's first migration against its tenth; a window taken over
 *  runs pooled from several tenants would compare one organisation's first
 *  against another's tenth and report a fall that happened to nobody. Nothing
 *  downstream could recover the error, because the resulting number is a
 *  plausible one. */
export interface DrainSeries {
	org: string;
	org_name: string;
	runs: DrainRun[];
	gate: GateWindow;
}

/** Every tenant's series, and how many rows were not readable as a
 *  measurement.
 *
 *  The count travels beside the series rather than being left to be inferred
 *  from it, because it cannot be: once a row is dropped, a tenant that
 *  recorded nothing and a tenant whose every row was malformed produce the
 *  same empty page, and those are opposite facts for whoever has to act on
 *  it. */
export interface DrainReadout {
	series: DrainSeries[];
	/** Rows `readMeasurement` refused. Zero is the ordinary case. */
	unreadable: number;
}

/** What the page shows when no series survived.
 *
 *  The two ways to reach an empty page are opposite facts that arrive in the
 *  same shape, so the copy has to be chosen from the count rather than from
 *  the series: nothing recorded is an ordinary quiet state, whereas every row
 *  refused is a fault, and announcing a fault as "none recorded yet" is the
 *  same silence this count exists to break. Icons are named as literals so
 *  this module stays free of the console's component types. */
export interface DrainEmptyState {
	icon: 'circle-check' | 'triangle-alert';
	headline: string;
	body: string;
}

export function emptyState(unreadable: number): DrainEmptyState {
	return unreadable === 0
		? {
				icon: 'circle-check',
				headline: 'No import has recorded a drain report yet',
				body: 'Each run records one, and the ledger keeps 30 days of them.'
			}
		: {
				icon: 'triangle-alert',
				headline: 'No drain report could be read',
				body: 'The rows are in the ledger, but none is in a shape this page can read.'
			};
}

/** The operator's view: every tenant's drain series, each measured on its own.
 *
 *  Organisations are ordered by name so the page is scannable and stable
 *  between reads; runs within one are ordered by ledger position, which is
 *  what makes "first" and "tenth" mean anything. */
export function seriesByOrg(rows: readonly DrainRow[]): DrainReadout {
	const groups = new Map<string, { org_name: string; runs: DrainRun[] }>();
	let unreadable = 0;
	for (const row of rows) {
		const measurement = readMeasurement(row.payload);
		if (measurement === null) {
			unreadable += 1;
			continue;
		}
		let group = groups.get(row.org);
		if (group === undefined) {
			group = { org_name: row.org_name, runs: [] };
			groups.set(row.org, group);
		}
		group.runs.push(toRun(row.org_seq, measurement));
	}
	const series = [...groups.entries()]
		.map(([org, group]) => {
			const runs = [...group.runs].sort((left, right) => left.seq - right.seq);
			return { org, org_name: group.org_name, runs, gate: gateWindow(runs) };
		})
		.sort((left, right) => left.org_name.localeCompare(right.org_name));
	return { series, unreadable };
}
