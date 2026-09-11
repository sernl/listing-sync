// Fixtures the Automations tests build their inputs from. Named `.test-support`
// rather than `.test` so vitest's own include pattern does not pick it up as a
// suite with no cases in it.

import type { ConnectionView, JobHead, QueueItem, SyncRequestHead } from '$lib/api';
import type { AuthorshipView } from '$lib/api';
import type { InventoryId, Marketplace } from '$lib/generated/vocab';

export function connection(
	marketplace: Marketplace,
	over: Partial<ConnectionView> = {}
): ConnectionView {
	return {
		id: `c-${marketplace}`,
		marketplace,
		state: 'linked',
		status: 'connected',
		created_at: 0,
		updated_at: 0,
		...over
	};
}

export const DECLARED: AuthorshipView = {
	state: 'declared',
	name: 'A Teacher',
	attested_at: 0
};

export function request(over: Partial<SyncRequestHead> = {}): SyncRequestHead {
	return {
		request: 'r-1',
		source: 'Tes',
		target: 'Tpt',
		disposition: 'migrate',
		intent: 'draft',
		state: 'enqueued',
		created_at: 0,
		resources_total: 4,
		resources_failed: 0,
		...over
	};
}

export function job(over: Partial<JobHead> = {}): JobHead {
	return { job: 'j-abcdef123456', inventory: 'Tpt', created_at: 0, ...over };
}

export function queued(over: Partial<QueueItem> = {}): QueueItem {
	return {
		id: 'q-1',
		term: 'Key Stage 3',
		inventory: 'Tpt' as InventoryId,
		kind: 'grade',
		raised_at: 0,
		...over
	};
}
