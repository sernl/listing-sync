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
	/** One product's aggregate, keyed by its identifier. */
	product: (id: string) => ['product', id] as const,
	/** One marketplace's authoring vocabulary, keyed by inventory: the create
	 *  form reads several at once and each is a separate cache entry, so
	 *  selecting a second platform fetches only the one it added. */
	vocabulary: (inventory: string) => ['vocabulary', inventory] as const,
	/** Every controlled list the canonical create form renders. One entry for
	 *  the whole form: it is the committed TPT capture rendered onto the wire
	 *  and changes only when the server does. */
	formVocabulary: ['form-vocabulary'] as const,
	/** The canonical terms of one kind. Keyed by the kind because the create
	 *  form reads the subjects alone, and a second kind is a second entry
	 *  rather than a refetch of this one. */
	taxonomyTerms: (kind: string) => ['taxonomy-terms', kind] as const,
	/** The newest runs' items, read so the inventory can say what is happening
	 *  to one listing on one marketplace. Its own key rather than `activity`:
	 *  that one holds job views, this one holds the items inside them, and one
	 *  key under two shapes is a cache collision. */
	inventoryWork: ['inventory-work'] as const,
	/** The seller's own machines. Shared by the dashboard's device band and by
	 *  the devices page, which is the one place a machine is signed out, so a
	 *  revoke there moves the band on the next read. */
	devices: ['devices'] as const,
	mappings: ['mappings'] as const,
	status: ['status'] as const,
	org: ['org'] as const,
	identity: ['identity'] as const,
	passkeys: ['passkeys'] as const,
	billing: ['billing'] as const,
	drainStats: ['drain-stats'] as const,
	/** The newest runs read in full, which the jobs list alone cannot give:
	 *  its heads carry no phase and no counts. */
	activity: ['job-activity'] as const,

	/** The operator probe. One read, shared by the sidebar that decides
	 *  whether to show the Admin group and by the `/admin` pages themselves,
	 *  so the question is asked once per session rather than once per page. */
	operator: ['operator-probe'] as const,
	/** The identity session as the impersonation banner reads it. A key of its
	 *  own rather than `identity`, which the settings page holds a differently
	 *  shaped answer under: one key, two shapes is a cache collision. */
	identitySession: ['identity-session'] as const,
	adminSignups: ['admin-signups'] as const,
	adminOrgs: ['admin-orgs'] as const,
	adminOrg: (org: string) => ['admin-org', org] as const,
	adminFailures: ['admin-failed-writes'] as const,
	adminImpersonations: ['admin-impersonations'] as const,
	/** The identity plane's user list, keyed by the search that produced it. */
	identityUsers: (search: string) => ['identity-users', search] as const
};
