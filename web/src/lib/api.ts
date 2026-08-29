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

// ------------------------------------------------------------------ shapes

export interface Whoami {
	org: string;
	user: string;
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

// --------------------------------------------------------------- endpoints

export const api = {
	whoami: () => request<Whoami>('/v1/whoami'),
	exchange: (token: string) => post<Whoami>('/v1/session', { token }),
	logout: () => request<void>('/v1/session', { method: 'DELETE' }),

	products: (cursor?: string | null) =>
		request<ProductsPage>(`/v1/products${cursor ? `?cursor=${encodeURIComponent(cursor)}` : ''}`),
	mappings: () => request<{ mappings: MappingHead[] }>('/v1/mappings'),

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

	status: () => request<{ inventories: InventoryStatus[] }>('/v1/status')
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
