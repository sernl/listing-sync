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
	{ label: 'done', pick: (c) => c.succeeded, tone: 'seg-ok' },
	{ label: 'done with warnings', pick: (c) => c.degraded, tone: 'seg-warn' },
	{ label: 'failed', pick: (c) => c.failed, tone: 'seg-bad' },
	{ label: 'unclear', pick: (c) => c.ambiguous, tone: 'seg-odd' },
	{ label: 'skipped', pick: (c) => c.skipped, tone: 'seg-mut' },
	{ label: 'blocked', pick: (c) => c.outcome_blocked + c.blocked, tone: 'seg-block' },
	{ label: 'sending', pick: (c) => c.in_flight, tone: 'seg-run' },
	{ label: 'held', pick: (c) => c.parked, tone: 'seg-park' },
	{ label: 'waiting', pick: (c) => c.queued, tone: 'seg-queue' }
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
