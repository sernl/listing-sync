// The typed API client: one fetch wrapper, the structured error parsed into
// a typed failure, and every endpoint the client consumes. Identifiers are
// hyphenated UUID strings and cursors are opaque server-minted tokens; this
// module never invents either.

import type {
	APIErrorCode,
	APIErrorKind,
	ConnectionStatus,
	FailureCode,
	InventoryId,
	ItemOutcome,
	ItemState,
	JobPhase,
	Marketplace
} from '$lib/generated/vocab';

export interface APIErrorEntry {
	code?: APIErrorCode;
	kind?: APIErrorKind;
	message: string;
	detail?: unknown;
}

export interface APIErrorBody {
	status: number;
	errors: APIErrorEntry[];
}

/** A failing response, thrown with its structured body when one existed. */
export class ApiFailure extends Error {
	readonly status: number;
	readonly body: APIErrorBody | null;

	constructor(status: number, body: APIErrorBody | null) {
		super(body?.errors[0]?.message ?? `request failed with ${status}`);
		this.status = status;
		this.body = body;
	}

	code(): APIErrorCode | undefined {
		return this.body?.errors[0]?.code;
	}
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
	const response = await fetch(path, {
		headers: { accept: 'application/json', ...(init?.headers ?? {}) },
		...init
	});
	if (response.status === 204) {
		return undefined as T;
	}
	if (!response.ok) {
		let body: APIErrorBody | null = null;
		try {
			body = (await response.json()) as APIErrorBody;
		} catch {
			body = null;
		}
		throw new ApiFailure(response.status, body);
	}
	return (await response.json()) as T;
}

function post<T>(path: string, body: unknown, headers?: Record<string, string>): Promise<T> {
	return request<T>(path, {
		method: 'POST',
		headers: { 'content-type': 'application/json', ...(headers ?? {}) },
		body: JSON.stringify(body)
	});
}

function patch<T>(path: string, body: unknown): Promise<T> {
	return request<T>(path, {
		method: 'PATCH',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify(body)
	});
}

// ------------------------------------------------------------------ shapes

export interface Whoami {
	org: string;
	user: string;
}

/** The organisation's own settings, as `/v1/org` serves and stores them. The
 *  name comes back from the rename too, because the server trims on the way in
 *  and a client that echoed what it sent would render a value it does not
 *  hold. */
export interface OrgView {
	id: string;
	name: string;
}

export interface ProductHead {
	id: string;
	title: string;
	price: unknown;
	created_at: number;
	updated_at: number;
}

export interface ProductsPage {
	products: ProductHead[];
	next_cursor: string | null;
}

export interface MappingHead {
	id: string;
	product: string;
	inventory: InventoryId;
	binding_state: string;
	lifecycle_state: string;
	updated_at: number;
}

export interface CreatedJob {
	job: string;
	replay: boolean;
}

export interface JobHead {
	job: string;
	inventory: InventoryId;
	created_at: number;
}

export interface JobsPage {
	jobs: JobHead[];
	next_cursor: string | null;
}

export interface Counts {
	total: number;
	queued: number;
	in_flight: number;
	blocked: number;
	parked: number;
	settled: number;
	succeeded: number;
	degraded: number;
	failed: number;
	ambiguous: number;
	skipped: number;
	outcome_blocked: number;
}

export interface JobView {
	job: string;
	inventory: InventoryId;
	created_at: number;
	phase: JobPhase;
	counts: Counts;
}

export interface ItemView {
	item: string;
	mapping: string;
	state: ItemState;
	outcome?: ItemOutcome;
	failure_code?: FailureCode;
	failure_detail?: string;
	blocked_on?: string;
	attempt_count: number;
	created_at: number;
	settled_at?: number;
}

export interface ItemsPage {
	items: ItemView[];
	next_cursor: string | null;
}

export interface EventView {
	org_seq: number;
	kind: string;
	payload: unknown;
	created_at: number;
}

export type ItemDetail = ItemView & { events: EventView[] };

export interface ConnectionView {
	id: string;
	marketplace: Marketplace;
	/// The stored link state. `status` is what the page renders.
	state: string;
	/// Whether the connection is actually carrying work, which the link
	/// state alone cannot say.
	status: ConnectionStatus;
	created_at: number;
	updated_at: number;
}

export interface QueueItem {
	id: string;
	term: string;
	inventory: InventoryId;
	kind: string;
	raised_at: number;
}

export interface DrainStats {
	open: number;
	resolved: number;
	no_counterpart: number;
}

export interface InventoryStatus {
	inventory: InventoryId;
	marketplace: Marketplace;
	halted: boolean;
	reason?: string;
	raised_at?: number;
}

/** One listing's newest captured figures.
 *
 *  `observed_at` is the *oldest* instant among the figures in this row, which
 *  the server chooses so that no number is presented fresher than it is.
 *
 *  `metrics` is keyed by the names the capture stored, deliberately open
 *  rather than a closed union: the captured shortlist can widen without a
 *  coordinated client release, and a client renders the keys it recognises. */
export interface ListingMetricsView {
	mapping: string;
	inventory: InventoryId;
	observed_at: number;
	metrics: Record<string, number>;
}

export interface AnalyticsSummary {
	listings: ListingMetricsView[];
}

/** One organisation's billing state, as `/v1/billing` serves it.
 *
 *  `status` is Paddle's own vocabulary, passed through by the server rather
 *  than translated, so a value this client does not recognise is displayed
 *  rather than swallowed. */
export interface SubscriptionView {
	paddle_subscription_id: string;
	paddle_customer_id: string;
	status: string;
	current_period_end: number | null;
	occurred_at: number;
}

/** Absent for an organisation that has never reached checkout, which is a
 *  different fact from a cancelled subscription: that one is present, and
 *  carries Paddle's cancelled status. */
export interface BillingView {
	subscription: SubscriptionView | null;
}

// ----------------------------------------------------------------- operator

/** One day and what was counted on it. The instant is the day's start in UTC,
 *  which is what the server's `date_trunc` returns. */
export interface DayCount {
	day: number;
	count: number;
}

/** Signups from both planes, newest day first.
 *
 *  `provisioned` counts rows in the platform's own `app_user` table, which the
 *  session exchange writes on a subject's first login. `identity` counts
 *  `user_signed_up` events in the identity service's audit trail, and is
 *  *absent* rather than empty where this deployment's database carries no
 *  identity schema — "nobody signed up" and "the identity trail is not visible
 *  from here" are different facts and the page says which one it is looking
 *  at. */
export interface SignupsView {
	provisioned: DayCount[];
	identity?: DayCount[];
}

export interface OrgSummaryView {
	org: string;
	name: string;
	created_at: number;
	products: number;
	mappings: number;
	connections: number;
	users: number;
}

export interface OrgsView {
	orgs: OrgSummaryView[];
}

/** A halt as recorded. `inventory` absent is the tenant-wide halt; present is
 *  the one raised against a single inventory. */
export interface HaltView {
	inventory?: InventoryId;
	reason: string;
	raised_by: string;
	raised_at: number;
}

/** What Paddle last said about one tenant's subscription, as the operator
 *  surface serves it: three facts and no Paddle identifier. */
export interface SubscriptionStateView {
	status: string;
	current_period_end?: number;
	occurred_at: number;
}

export interface OrgDetailView {
	org: OrgSummaryView;
	connections: ConnectionView[];
	halts: HaltView[];
	subscription?: SubscriptionStateView;
}

/** The ledger across every tenant, in the stored state vocabulary rather than
 *  the collapsed one a seller's job page renders: whether items are parked
 *  live or parked cold is the distinction an operator opened the page to
 *  find. */
export interface SyncHealthView {
	jobs: number;
	items: number;
	queued: number;
	leased: number;
	running: number;
	blocked: number;
	parked_live: number;
	parked_cold: number;
	verifying: number;
	settled: number;
	succeeded: number;
	degraded: number;
	failed: number;
	ambiguous: number;
	skipped: number;
	outcome_blocked: number;
}

export interface FailedWriteView {
	org: string;
	attempt: string;
	item: string;
	mapping: string;
	state: string;
	opened_at: number;
	settled_at?: number;
	failure_code: FailureCode;
	ambiguity_cause?: string;
	item_failure_code?: FailureCode;
	item_failure_detail?: string;
}

export interface FailedWritesView {
	writes: FailedWriteView[];
}

/** One impersonation as the identity service recorded it. `actor` and
 *  `target` are identity-plane subject ids, not platform user ids: the two
 *  planes number their users separately. */
export interface ImpersonationView {
	event: string;
	actor: string;
	target: string;
	at: number;
	ip?: string;
}

/** `impersonations` is absent rather than empty where the identity schema is
 *  not visible, for the reason `SignupsView.identity` is. */
export interface ImpersonationsView {
	impersonations?: ImpersonationView[];
}

// --------------------------------------------------------------- endpoints

export const api = {
	whoami: () => request<Whoami>('/v1/whoami'),
	exchange: (token: string) => post<Whoami>('/v1/session', { token }),
	logout: () => request<void>('/v1/session', { method: 'DELETE' }),

	org: () => request<OrgView>('/v1/org'),
	renameOrg: (name: string) => patch<OrgView>('/v1/org', { name }),

	products: (cursor?: string | null) =>
		request<ProductsPage>(`/v1/products${cursor ? `?cursor=${encodeURIComponent(cursor)}` : ''}`),
	mappings: () => request<{ mappings: MappingHead[] }>('/v1/mappings'),
	analytics: () => request<AnalyticsSummary>('/v1/analytics/summary'),

	createJob: (inventory: InventoryId, mappings: string[], idempotencyKey: string) =>
		post<CreatedJob>(
			'/v1/jobs',
			{ inventory, mappings },
			{ 'idempotency-key': idempotencyKey }
		),
	jobs: (cursor?: string | null) =>
		request<JobsPage>(`/v1/jobs${cursor ? `?cursor=${encodeURIComponent(cursor)}` : ''}`),
	job: (id: string) => request<JobView>(`/v1/jobs/${id}`),
	items: (job: string, cursor?: string | null) =>
		request<ItemsPage>(
			`/v1/jobs/${job}/items${cursor ? `?cursor=${encodeURIComponent(cursor)}` : ''}`
		),
	item: (job: string, item: string) => request<ItemDetail>(`/v1/jobs/${job}/items/${item}`),

	connections: () => request<{ connections: ConnectionView[] }>('/v1/connections'),
	revoke: (connection: string) =>
		post<{ connections: number; elapsed_ms: number }>(
			`/v1/connections/${connection}/revoke`,
			{}
		),

	queue: () => request<{ items: QueueItem[] }>('/v1/reconciliation/items'),
	resolve: (item: string, segments: string[], nativeId?: string) =>
		post<void>(`/v1/reconciliation/items/${item}/resolve`, {
			segments,
			native_id: nativeId ?? null
		}),
	noCounterpart: (item: string) =>
		post<void>(`/v1/reconciliation/items/${item}/no-counterpart`, {}),
	drainStats: () => request<DrainStats>('/v1/reconciliation/stats'),

	status: () => request<{ inventories: InventoryStatus[] }>('/v1/status'),

	billing: () => request<BillingView>('/v1/billing'),

	// The operator surface. Every one of these answers a blank 401 to a caller
	// who is not an operator — the same body an anonymous request gets, so the
	// refusal is never an oracle. A client reading one treats it as "not an
	// operator", never as a fault.
	adminSignups: () => request<SignupsView>('/v1/admin/signups'),
	adminOrgs: () => request<OrgsView>('/v1/admin/orgs'),
	adminOrg: (org: string) => request<OrgDetailView>(`/v1/admin/orgs/${org}`),
	adminSyncHealth: () => request<SyncHealthView>('/v1/admin/sync-health'),
	adminFailedWrites: () => request<FailedWritesView>('/v1/admin/failed-writes'),
	adminImpersonations: () => request<ImpersonationsView>('/v1/admin/impersonations')
};

/** Walks every page of a cursor-paginated endpoint, accumulating rows. */
export async function allPages<Row, Page extends { next_cursor: string | null }>(
	fetchPage: (cursor?: string | null) => Promise<Page>,
	rows: (page: Page) => Row[]
): Promise<Row[]> {
	const collected: Row[] = [];
	let cursor: string | null | undefined = undefined;
	for (;;) {
		const page = await fetchPage(cursor);
		collected.push(...rows(page));
		if (!page.next_cursor) {
			return collected;
		}
		cursor = page.next_cursor;
	}
}
