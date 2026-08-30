// The dashboard's figures, each one derived from a field the API actually
// serves. Nothing here interpolates, projects or compares against a previous
// period: no endpoint carries a time series, so the overview states what is
// recorded now and omits what would have to be invented. Pure, so it tests
// without a component.

import { capturedAgo } from '$lib/analytics-view';
import type {
	ConnectionView,
	InventoryStatus,
	JobView,
	ListingMetricsView,
	MappingHead,
	ProductHead
} from '$lib/api';
import { present } from '$lib/connection-status';
import { MARKETPLACE_OF } from '$lib/listings-view';
import { segments } from '$lib/outcome';
import { standingOf } from '$lib/tes-portfolio';

/** How many of the newest runs the overview reads in full. The jobs list
 *  serves heads without a phase or counts, so each of these costs a read of
 *  its own and the panel is bounded rather than paginated. */
export const ACTIVITY_LIMIT = 6;

/** How many rows the recently-updated strip shows. */
export const RECENT_LIMIT = 5;

export interface LiveTally {
	/** Mappings bound to a listing the marketplace was last recorded showing. */
	live: number;
	/** Every mapping, whatever standing it is in. */
	total: number;
	/** Distinct marketplaces carrying at least one live listing. */
	marketplaces: number;
}

export function liveTally(mappings: readonly MappingHead[]): LiveTally {
	const marketplaces = new Set<string>();
	let live = 0;
	for (const mapping of mappings) {
		if (standingOf(mapping) === 'live') {
			live += 1;
			marketplaces.add(MARKETPLACE_OF[mapping.inventory]);
		}
	}
	return { live, total: mappings.length, marketplaces: marketplaces.size };
}

export type ActivityTone = 'ok' | 'run' | 'bad' | 'mut';

export interface ActivityRow {
	job: string;
	inventory: string;
	tone: ActivityTone;
	label: string;
	/** The run's own counts, written out; never a verdict of our own. */
	detail: string;
	created_at: number;
}

/** What one run is doing, or what it settled as.
 *
 * A settled run is named by the worst outcome it recorded rather than by a
 * pass or fail: the ledger keeps every outcome separately, and collapsing
 * them into a verdict would hide the ones a seller can still act on. */
function outcomeOf(job: JobView): { tone: ActivityTone; label: string } {
	if (job.phase === 'active') {
		return { tone: 'run', label: 'Running' };
	}
	if (job.counts.failed > 0) {
		return { tone: 'bad', label: 'Failed' };
	}
	if (job.counts.blocked + job.counts.outcome_blocked > 0) {
		return { tone: 'run', label: 'Blocked' };
	}
	if (job.counts.degraded + job.counts.ambiguous > 0) {
		return { tone: 'run', label: 'Degraded' };
	}
	if (job.counts.total === 0) {
		return { tone: 'mut', label: 'Empty' };
	}
	return { tone: 'ok', label: 'Done' };
}

export function activityRows(jobs: readonly JobView[]): ActivityRow[] {
	return jobs.map((job) => {
		const written = segments(job.counts)
			.map((segment) => `${segment.count} ${segment.label}`)
			.join(' · ');
		return {
			job: job.job,
			inventory: job.inventory,
			created_at: job.created_at,
			...outcomeOf(job),
			detail: written.length > 0 ? `${job.counts.total} items · ${written}` : 'no items'
		};
	});
}

export interface SyncTally {
	/** Runs among those read that have not settled. */
	active: number;
	/** Items in those runs queued or in flight. */
	pending: number;
	/** Items that failed, across the runs read. */
	failed: number;
	/** Runs carrying at least one failed item. */
	failingRuns: number;
	/** How many runs these figures were counted over. */
	runs: number;
}

export function syncTally(jobs: readonly JobView[]): SyncTally {
	const tally: SyncTally = { active: 0, pending: 0, failed: 0, failingRuns: 0, runs: jobs.length };
	for (const job of jobs) {
		if (job.phase === 'active') {
			tally.active += 1;
		}
		tally.pending += job.counts.queued + job.counts.in_flight;
		tally.failed += job.counts.failed;
		if (job.counts.failed > 0) {
			tally.failingRuns += 1;
		}
	}
	return tally;
}

export interface SalesCapture {
	/** The captured sales total, or null when no listing carries one. */
	sales: number | null;
	/** Listings the capture covered. */
	listings: number;
	/** The age of the oldest figure in the total, in the analytics page's own
	 *  words, or null when there is nothing captured to age. */
	age: string | null;
}

/** The captured sales total and how old it is.
 *
 * The age is the oldest instant among the rows summed, not the newest: a
 * total is only as fresh as its stalest part, and stating otherwise would
 * make a figure read live that is not. */
export function salesCapture(
	listings: readonly ListingMetricsView[],
	now: number
): SalesCapture {
	let sales: number | null = null;
	let oldest: number | null = null;
	for (const listing of listings) {
		const captured = listing.metrics.sales_count;
		if (typeof captured === 'number' && Number.isFinite(captured)) {
			sales = (sales ?? 0) + captured;
		}
		if (oldest === null || listing.observed_at < oldest) {
			oldest = listing.observed_at;
		}
	}
	return {
		sales,
		listings: listings.length,
		age: oldest === null ? null : capturedAgo(oldest, now)
	};
}

export type AttentionTone = 'bad' | 'warn';

export interface AttentionItem {
	key: string;
	tone: AttentionTone;
	title: string;
	body: string;
	action: { label: string; href: string };
}

export interface AttentionInput {
	connections: readonly ConnectionView[];
	statuses: readonly InventoryStatus[];
	openQuestions: number;
	tally: SyncTally;
}

function plural(count: number, one: string, many: string): string {
	return count === 1 ? one : many;
}

/**
 * Only what a person has to decide, in the order it costs them.
 *
 * Every entry names a signal the API serves: a connection's own status, a
 * halted inventory's own reason, the failed count on a run, and the open
 * count on the reconciliation queue. There is no entry that a payload did not
 * put there.
 */
export function attention(input: AttentionInput): AttentionItem[] {
	const items: AttentionItem[] = [];
	for (const connection of input.connections) {
		if (connection.status === 'disconnected') {
			items.push({
				key: `connection-${connection.id}`,
				tone: 'bad',
				title: `Reconnect ${connection.marketplace}`,
				body: present(connection.status).explanation,
				action: { label: 'Open connections', href: '/connections' }
			});
		}
	}
	for (const status of input.statuses) {
		if (status.halted) {
			items.push({
				key: `halted-${status.inventory}`,
				tone: 'bad',
				title: `${status.inventory} is halted`,
				body:
					status.reason ??
					'Work for this inventory is paused. Nothing is lost; queued items wait.',
				action: { label: 'Open status', href: '/status' }
			});
		}
	}
	if (input.tally.failed > 0) {
		const { failed, failingRuns } = input.tally;
		items.push({
			key: 'failed-writes',
			tone: 'bad',
			title: `${failed} ${plural(failed, 'write', 'writes')} failed`,
			body: `Across ${failingRuns} of the ${input.tally.runs} most recent ${plural(
				input.tally.runs,
				'run',
				'runs'
			)}. Each failure keeps its own reason on the run it belongs to.`,
			action: { label: 'Open sync', href: '/sync' }
		});
	}
	for (const connection of input.connections) {
		if (connection.status === 'unstable') {
			items.push({
				key: `unstable-${connection.id}`,
				tone: 'warn',
				title: `${connection.marketplace} is unstable`,
				body: present(connection.status).explanation,
				action: { label: 'Open connections', href: '/connections' }
			});
		}
	}
	if (input.openQuestions > 0) {
		items.push({
			key: 'reconciliation',
			tone: 'warn',
			title: `${input.openQuestions} ${plural(
				input.openQuestions,
				'question',
				'questions'
			)} waiting`,
			body: 'A listing carries a term with no translation yet, and each one you answer stays answered.',
			action: { label: 'Answer in Reconciliation', href: '/queue' }
		});
	}
	return items;
}

/** The most recently touched products, newest first. */
export function recentlyUpdated(
	products: readonly ProductHead[],
	limit: number
): ProductHead[] {
	return [...products].sort((left, right) => right.updated_at - left.updated_at).slice(0, limit);
}
