// One QueryClient for the app: the REST reads under /v1 go through it, and
// the progress stream deliberately does not. The stream is Server-Sent Events
// consumed by the browser's own EventSource, which owns its reconnection and
// its Last-Event-ID cursor (`$lib/ledger`); wrapping that in a query cache
// would replace working resumption with polling.

import { QueryClient } from '@tanstack/svelte-query';
import { ApiFailure } from '$lib/api';

const MAX_RETRIES = 3;

/** A 401 is the session guard's signal, not a transient fault: retrying it
 * only delays the redirect to sign-in. Every other failure keeps the default
 * backoff. */
function retryUnlessUnauthenticated(failures: number, failure: Error): boolean {
	if (failure instanceof ApiFailure && failure.status === 401) {
		return false;
	}
	return failures < MAX_RETRIES;
}

export function createQueryClient(): QueryClient {
	return new QueryClient({
		defaultOptions: { queries: { retry: retryUnlessUnauthenticated } }
	});
}

/** The cache keys, named once so an invalidation and its query cannot drift. */
export const queryKeys = {
	connections: ['connections'] as const,
	analytics: ['analytics'] as const,
	products: ['products'] as const,
	mappings: ['mappings'] as const,
	status: ['status'] as const,
	org: ['org'] as const,
	identity: ['identity'] as const,
	passkeys: ['passkeys'] as const,
	billing: ['billing'] as const,
	drainStats: ['drain-stats'] as const,
	/** The newest runs read in full, which the jobs list alone cannot give:
	 *  its heads carry no phase and no counts. */
	activity: ['job-activity'] as const
};
