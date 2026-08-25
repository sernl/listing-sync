// The outcome bar's arithmetic: raw counts to proportional segments, every
// segment the ledger can store and no scalar verdict. Pure, so it tests
// without a component.

import type { Counts } from '$lib/api';

export interface Segment {
	label: string;
	count: number;
	share: number;
	tone: string;
}

const SEGMENTS: Array<{ label: string; pick: (c: Counts) => number; tone: string }> = [
	{ label: 'succeeded', pick: (c) => c.succeeded, tone: 'bg-emerald-500' },
	{ label: 'degraded', pick: (c) => c.degraded, tone: 'bg-amber-500' },
	{ label: 'failed', pick: (c) => c.failed, tone: 'bg-red-500' },
	{ label: 'ambiguous', pick: (c) => c.ambiguous, tone: 'bg-purple-500' },
	{ label: 'skipped', pick: (c) => c.skipped, tone: 'bg-slate-400' },
	{ label: 'blocked', pick: (c) => c.outcome_blocked + c.blocked, tone: 'bg-orange-500' },
	{ label: 'in flight', pick: (c) => c.in_flight, tone: 'bg-sky-500' },
	{ label: 'parked', pick: (c) => c.parked, tone: 'bg-slate-500' },
	{ label: 'queued', pick: (c) => c.queued, tone: 'bg-slate-300' }
];

export function segments(counts: Counts): Segment[] {
	const total = Math.max(counts.total, 1);
	return SEGMENTS.map(({ label, pick, tone }) => ({
		label,
		count: pick(counts),
		share: pick(counts) / total,
		tone
	})).filter((segment) => segment.count > 0);
}
