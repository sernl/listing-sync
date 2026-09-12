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
	/** The catalogue narrowed to one label. Its own key, because the label is
	 *  part of what the server was asked for rather than a view over one
	 *  answer. */
	catalogue: (label: string | null) => ['products', label] as const,
	labels: ['labels'] as const,
	/** The published release manifest the Marketplaces page's download cards
	 *  read. Served from our own origin rather than the API, and cached like
	 *  every other read so the key lives here with them. */
	downloadsManifest: ['downloads-manifest'] as const,
	/** The seller's own projection overrides, which the Templates screen reads
	 *  and writes. */
	overrides: ['overrides'] as const,
	/** One product's aggregate, keyed by its identifier. */
	product: (id: string) => ['product', id] as const,
	/** The labels on one item, which is a different read from the
	 *  organisation's whole vocabulary above: this one carries the marks an
	 *  import wrote on that item. */
	productLabels: (id: string) => ['product', id, 'labels'] as const,
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
	 *  the Marketplaces screen, which is the one place a machine is signed out,
	 *  so a revoke there moves the band on the next read. */
	devices: ['devices'] as const,
	/** The identity plane's browser sign-ins. Shared because the two screens
	 *  that read them can each end one: Account Settings lists them, and the
	 *  Marketplaces screen ends the matched sign-in when it signs a machine
	 *  out. */
	browserSessions: ['browser-sessions'] as const,
	/** The token of the sign-in this browser is using, so a row whose sign-out
	 *  would end the session being read can say so. */
	currentSessionToken: ['current-session-token'] as const,
	mappings: ['mappings'] as const,
	status: ['status'] as const,
	org: ['org'] as const,
	orgSlug: (slug: string) => ['org-slug', slug] as const,
	identity: ['identity'] as const,
	/** Whether this seller takes email when a run finishes. Read on the
	 *  preferences screen alone; the rail carries no unread badge, so
	 *  nothing else asks. */
	notifyPreferences: ['notify-preferences'] as const,
	/** The seller's own picture. Read by the shell, which draws it in the top
	 *  strip and the phone bar, and set by the preferences screen from the
	 *  server's answer, so the tile moves without a refetch. */
	profile: ['profile'] as const,
	passkeys: ['passkeys'] as const,
	billing: ['billing'] as const,
	/** What this organisation's plan allows and what it has used. Asked once
	 *  by the shell and read from the cache by every page that draws a gated
	 *  control, so a cap is stated the same way everywhere on one answer. */
	entitlement: ['entitlement'] as const,
	/** The price table. Served publicly and identical for every caller, so it
	 *  is one entry with no tenant in its key. */
	plans: ['plans'] as const,
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
	adminImportDrain: ['admin-import-drain'] as const,
	adminDeadLetters: ['admin-dead-letters'] as const,
	adminImpersonations: ['admin-impersonations'] as const,
	/** The identity plane's user list, keyed by the search that produced it. */
	identityUsers: (search: string) => ['identity-users', search] as const
};
