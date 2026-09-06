// The typed API client: one fetch wrapper, the structured error parsed into
// a typed failure, and every endpoint the client consumes. Identifiers are
// hyphenated UUID strings and cursors are opaque server-minted tokens; this
// module never invents either.

import type {
	APIErrorCode,
	APIErrorKind,
	BodyWire,
	Cardinality,
	ConnectionStatus,
	CopyFormat,
	Delegation,
	DeviceSessionStatus,
	ElectionTriggerKind,
	FailureCode,
	FileKind,
	FileRole,
	InventoryId,
	ItemOutcome,
	ItemState,
	JobPhase,
	LengthUnit,
	Marketplace,
	NativeDirection,
	NativeVocabularyKind,
	NonDelegableReason,
	PayloadFileRule,
	FormGroup,
	SlugPrompt,
	StandardsState,
	TermKind
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
		// `errors?.[0]` rather than `errors[0]`: the optional chain has to guard
		// the array too. A failing response whose body parses but carries no
		// `errors` — a bare `{}` — threw a TypeError here, inside the
		// constructor, so the ApiFailure was never built and the throw escaped
		// every `catch` that tests for one, taking the app to its 500 page.
		super(body?.errors?.[0]?.message ?? `request failed with ${status}`);
		this.status = status;
		this.body = body;
	}

	code(): APIErrorCode | undefined {
		return this.body?.errors?.[0]?.code;
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

function put<T>(path: string, body: unknown): Promise<T> {
	return request<T>(path, {
		method: 'PUT',
		headers: { 'content-type': 'application/json' },
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

/** Who the session speaks for. It carries the slug and the prompt that slug
 *  calls for because the console's gate is answered here: `+layout.ts` calls
 *  `/v1/whoami` before anything renders, so a seller who has claimed no slug
 *  meets the claim screen rather than a console they would be sent away from. */
export interface Whoami {
	org: string;
	user: string;
	slug: string | null;
	slug_prompt: SlugPrompt;
}

/** The organisation's own settings, as `/v1/org` serves and stores them. The
 *  name comes back from the rename too, because the server trims on the way in
 *  and a client that echoed what it sent would render a value it does not
 *  hold. */
export interface OrgView {
	id: string;
	name: string;
	/** The unique, URL-safe handle the seller claimed, or `null` while they have
	 *  claimed none. Distinct from `name`, which is what they want printed. */
	slug: string | null;
	slug_prompt: SlugPrompt;
}

/** A verdict on one slug, not an organisation, which is why it names no
 *  identifier. `slug` is the normalised form, so the screen can show the seller
 *  what they would actually get.
 *
 *  Advisory. The write's 409 stays authoritative, because check-then-write is a
 *  race only the server's unique index settles. */
export interface SlugAvailability {
	slug: string;
	available: boolean;
}

/** One marketplace session a device holds, as the device last reported it.
 *  Metadata only: migration 0042 has no column a cookie could travel in, which
 *  is decision D1 made structural rather than remembered. */
export interface DeviceSessionView {
	marketplace: Marketplace;
	account_label: string | null;
	linked_at: number;
	last_used_at: number;
	status: DeviceSessionStatus;
}

/** One of the seller's own machines. `wipe_outstanding` is the honest half of
 *  D14: the device was signed out and has not checked in since, so it may
 *  still hold the marketplace cookies listed beside it. */
export interface DeviceView {
	id: string;
	name: string;
	os: string;
	arch: string;
	app_version: string;
	first_seen_at: number;
	last_seen_at: number;
	revoked_at: number | null;
	wipe_outstanding: boolean;
	sessions: DeviceSessionView[];
}

export interface DevicesView {
	devices: DeviceView[];
}

export interface ProductHead {
	id: string;
	title: string;
	price: unknown;
	/** Where this resource's cover is fetched from, absent where it has none.
	 *  The server names the URL rather than the client composing it, so the
	 *  route is written down in one place. */
	cover?: string | null;
	created_at: number;
	updated_at: number;
}

export interface ProductsPage {
	products: ProductHead[];
	next_cursor: string | null;
}

/** One seller-defined label. The colour is the server's, derived from the
 *  name, so one label looks the same everywhere it appears. */
export interface LabelView {
	name: string;
	colour: string;
}

export interface LabelsView {
	labels: LabelView[];
}

/** One seller's own answer for how a term of theirs projects onto a
 *  marketplace, as `GET /v1/mappings/overrides` serves it. The term arrives as
 *  its identifier; the screen resolves the word from the taxonomy it already
 *  reads. */
export interface OverrideView {
	inventory: InventoryId;
	axis: TermKind;
	from_term: string;
	segments: string[];
	native_id?: string;
	kind: 'exact' | 'broader';
	decided_at: number;
}

export interface OverridesView {
	overrides: OverrideView[];
}

/** What an override write names. The organisation and the user are absent on
 *  purpose: the server takes both from the session, so a body cannot speak for
 *  an organisation it does not belong to. */
export interface OverrideInput {
	inventory: InventoryId;
	axis: TermKind;
	from_term: string;
	to: { segments: string[]; native_id?: string };
	kind: 'exact' | 'broader';
}

/** Which override to withdraw: one organisation holds at most one per
 *  marketplace, axis and term, so the triple is the key. */
export interface WithdrawOverrideInput {
	inventory: InventoryId;
	axis: TermKind;
	from_term: string;
}

export interface MappingHead {
	id: string;
	product: string;
	inventory: InventoryId;
	binding_state: string;
	lifecycle_state: string;
	updated_at: number;
	/** The listing's own page on the marketplace, derived by the server from
	 *  the identifier the binding holds. Null while the mapping binds nothing,
	 *  and null for a marketplace whose page shape the server has not
	 *  observed. */
	listing_url: string | null;
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

/** The term counters one measurement against the canonical taxonomy produced.
 *
 *  Optional wherever it is carried, and absent and present-with-zeros are two
 *  different facts: absent means nothing was measured there, and zeros mean a
 *  measurement was taken and found nothing. A client that read the absent case
 *  as zeros would state a measurement nobody took. Null and undefined are both
 *  spellings of absent. */
export interface SyncTermCoverage {
	terms_seen: number;
	terms_mapped: number;
	terms_unmapped: number;
	terms_uncovered: number;
}

/** The request's own coverage: the term counters summed across the resources
 *  the server measured, and how many resources that sum is over.
 *
 *  Present exactly when the request's source is a device-branch marketplace,
 *  carrying zeros for a catalogue that turned out to be empty; absent for a
 *  sync, and for a migrate whose source runs on our own infrastructure. */
export interface SyncCoverageView extends SyncTermCoverage {
	rows: number;
}

/** Where a request stands. The set is closed by `sync_request_state` in
 *  migration 0025, which is applied and frozen, so it is the database's own
 *  vocabulary rather than a guess made here. */
export type SyncRequestState = 'pending' | 'draining' | 'enqueued' | 'failed';

/** Closed by `sync_request_resource_state` in the same migration. */
export type SyncResourceState = 'pending' | 'canonicalised' | 'failed';

/** One listing the request names, as the request last recorded it.
 *
 *  A resource the seller's device skipped arrives `failed` carrying the
 *  device's own words in `failure_detail`, which are rendered verbatim: this
 *  client never re-words a marketplace's or a device's refusal.
 *
 *  `coverage` is present only for a resource the server measured, so it is
 *  absent on a skipped one and on every resource of a request that measures
 *  nothing. It carries the term counters alone: the resource is the row, so a
 *  row count on it would count itself. */
export interface SyncResourceView {
	ordinal: number;
	locator: string;
	state: SyncResourceState;
	failure_detail: string | null;
	coverage?: SyncTermCoverage | null;
}

/** A sync or migrate request as it fills.
 *
 *  `create_job` is null while nothing has been enqueued, and stays null when
 *  the catalogue turned out to be empty. That is why an `enqueued` request
 *  with no resources is a completed import of an empty shop rather than a
 *  failure, and the two must never render alike.
 *
 *  `waiting_for_device_version` is present while no device this seller has
 *  registered is new enough to run the work, and absent otherwise. */
export interface SyncRequestView {
	request: string;
	source: InventoryId;
	target: InventoryId;
	disposition: string;
	intent: string;
	state: SyncRequestState;
	failure_detail: string | null;
	create_job: string | null;
	remove_job: string | null;
	resources: SyncResourceView[];
	coverage?: SyncCoverageView | null;
	waiting_for_device_version?: string | null;
}

/** One request as the list serves it: enough to name it, place it in time and
 *  link to it, without the resource rows the detail view carries.
 *
 *  `resources_total` and `resources_failed` are counts rather than rows,
 *  because the list exists so that a request created on one machine is
 *  reachable from another, not so that it can be read in full from here. */
export interface SyncRequestHead {
	request: string;
	source: InventoryId;
	target: InventoryId;
	disposition: string;
	intent: string;
	state: SyncRequestState;
	created_at: number;
	resources_total: number;
	resources_failed: number;
}

/** Newest first, at most fifty. No cursor: the endpoint is bounded rather than
 *  paginated, so a seller with more than fifty requests sees the newest fifty
 *  and this client invents no way to ask for the rest. */
export interface SyncRequestsView {
	requests: SyncRequestHead[];
}

/** What the server answers a submit with: the request's identity, which is
 *  the idempotency key it was sent under. */
export interface SyncRequestAck {
	request: string;
}

/** A submit.
 *
 *  `resources` is empty for a migrate whose source runs on the seller's own
 *  device, and that is required rather than merely allowed: the device
 *  enumerates the catalogue, so the server refuses such a migrate that names
 *  resources as well as every other request that names none. There is no
 *  separate request kind for it — the empty list is what says so. */
export interface SyncRequestBody {
	source: InventoryId;
	target: InventoryId;
	disposition: 'sync' | 'migrate';
	intent: PublishIntent;
	resources: string[];
}

/** What a surface knows about the seller's declaration for one marketplace.
 *
 *  Three states, and the third is this field being absent altogether. A seller
 *  who has not declared is `undeclared`, which is a fact about them; a surface
 *  that does not serve declarations omits the field, which is a fact about the
 *  surface. Never render "not declared" on an absent field: on the operator's
 *  view that would state something false about every seller.
 *
 *  Read whatever state the connection is in: a declaration made before any
 *  device linked, or standing while the connection needs a fresh sign-in, is
 *  still on record and the seller looking to check it must see it. */
export type AuthorshipView =
	| { state: 'undeclared' }
	| { state: 'declared'; name: string; attested_at: number };

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
	/// The seller's own authorship declaration for this marketplace, absent
	/// where none stands.
	authorship?: AuthorshipView;
}

/** The connections answer as `ConnectionsView` in
 *  `crates/tam-api/src/resources.rs` serialises it: the list inside an
 *  envelope, never a bare array. */
export interface ConnectionsView {
	connections: ConnectionView[];
}

/** The list out of the envelope, which is the only shape this module lets out.
 *
 *  The envelope escaping here is what blanked the Resources board: two query
 *  functions under the one `['connections']` cache key held two shapes, and
 *  whichever refetched last decided which one the board was handed, so
 *  `rowFor` called `.find` on an object. Unwrapping at the wire leaves one
 *  shape for every caller to hold.
 *
 *  An answer that is not the envelope raises rather than degrading to `[]`:
 *  the pages tell an unread list apart from an empty one, and a silent `[]`
 *  would tell a seller they are signed in nowhere. */
export function connectionList(answer: unknown): ConnectionView[] {
	const held = (answer as ConnectionsView | null | undefined)?.connections;
	if (!Array.isArray(held)) {
		throw new Error('/v1/connections answered something other than a list of connections');
	}
	return held;
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
	/** The handle the tenant claimed, or `null` while they have claimed none.
	 *  What lets an operator find a tenant from a slug quoted in a support
	 *  email, which the provisional `org-{uuid}` name never allowed. */
	slug: string | null;
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

/** One import-drain measurement as the operator route serves it: which tenant
 *  wrote it, where in that tenant's ledger it sits, and the measurement itself
 *  verbatim.
 *
 *  `payload` is `unknown` because it is `jsonb` at rest and the route passes it
 *  through unparsed; `$lib/drain` narrows it and drops a row that is not a
 *  measurement, so one malformed historical row cannot empty the page. */
export interface ImportDrainRowView {
	org: string;
	org_name: string;
	org_seq: number;
	payload: unknown;
}

export interface ImportDrainView {
	rows: ImportDrainRowView[];
}

export interface FailedWriteView {
	org: string;
	attempt: string;
	item: string;
	mapping: string;
	state: string;
	opened_at: number;
	settled_at?: number;
	failure_code?: FailureCode;
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

// ---------------------------------------------------------------- authoring

/** Where the seller asked a publish to leave the listing. The server's own
 *  default is `draft`, and `live` is a deliberate second choice. */
export type PublishIntent = 'draft' | 'live';


/** A price as `PriceIntent` crosses the wire: the bare string, or the tagged
 *  object carrying the amount in the denomination's minor units.
 *
 *  The read side of the catalogue still serves this as `unknown` (see
 *  `ProductHead.price`) because `formatPrice` is written to refuse a shape it
 *  does not recognise rather than guess at one. The write side is typed,
 *  because a body this client composes is a body it fully controls. */
export type PriceIntent = 'Free' | { Paid: { minor_units: number; currency: string } };

/** One byte handle as `POST /v1/uploads` returns it and a create names back.
 *  The hash is the whole handle: bytes are content-addressed per tenant, so a
 *  hash this organisation never stored resolves to no blob and the create is
 *  refused. */
export interface FileHandle {
	hash: string;
	kind: FileKind;
	byte_len: number;
	/** What the seller called the file they chose. The upload reads a raw body
	 *  and a raw body carries no filename, so this is the client's word,
	 *  carried back beside the handle rather than returned by the upload. */
	name?: string;
}

/** What one upload landed, plus this tenant's storage headroom, which the
 *  server reports so a client renders it without a second call. */
export interface UploadedView {
	payload: FileHandle[];
	cover: FileHandle;
	previews: FileHandle[];
	stored_bytes: number;
	storage_bytes_max: number;
}

/** How an archive upload is treated. `keep_whole` is the mode a bundle bound
 *  for TPT needs, because a TPT create takes exactly one file. */
export type ArchiveMode = 'explode' | 'keep_whole';

/** One vocabulary value as a create or an edit names it, matching the shape
 *  `PathView` reads back. */
export interface PathInput {
	inventory: InventoryId;
	kind: TermKind;
	segments: string[];
	native_id?: string | null;
}

/** The rights grant the seller stated. No axis is carried: a rights
 *  declaration is a licence by construction. */
export interface RightsInput {
	inventory: InventoryId;
	segments: string[];
	native_id?: string | null;
}

export interface AnswerInput {
	segments: string[];
	native_id?: string | null;
}

/** One answer the seller gave on the form, before any mapping exists.
 *
 *  `trigger_key` is `free` or `paid` for a supply and the source value's
 *  native id for a narrow; the other two kinds generalise to nothing and the
 *  server refuses a key on them. */
export interface ElectionInput {
	inventory: InventoryId;
	axis: TermKind;
	trigger: ElectionTriggerKind;
	trigger_key?: string | null;
	answers: AnswerInput[];
}

/** The TPT-base fields `product` has no column for, which land in the sidecar
 *  migration 0040 declares. Deliberately a subset: the title, description,
 *  price, payload handles and grades travel on the create body itself, and a
 *  second copy of any of them is a second thing to disagree with. */
export interface TptBaseInput {
	thumbnail_mode?: number | null;
	thumbnail_hashes?: string[];
	video_preview_hash?: string | null;
	additional_licence_minor_units?: number | null;
	bundle_discount_minor_units?: number | null;
	tax_code_id?: number | null;
	subject_areas?: string[];
	tags?: string[];
	formats?: string[];
	custom_categories?: string[];
	/** `data[ItemsLocalization][country_id_flag]`. Absent or null states nothing
	 *  about the control rather than answering it: the sidecar holds three
	 *  states, and a form that rendered the checkbox sends the box's own state
	 *  either way, so false arrives only as an answer. `null` travels because an
	 *  edit form seeded from a product that never answered has to be able to
	 *  leave it unanswered; `Option<bool>` on the server reads it as `None`. */
	appropriate_for_country?: boolean | null;
	standards?: { framework: number; code: string; tpt_node_id?: number | null }[];
	teaching_duration_id?: number | null;
	pages_or_slides?: number | null;
	answer_key_id?: number | null;
	copyright_declaration_id?: number | null;
	status_user?: number | null;
}

export interface CreateProductBody {
	title: string;
	body?: string;
	body_format?: CopyFormat;
	price: PriceIntent;
	payload: FileHandle[];
	cover?: FileHandle | null;
	previews?: FileHandle[];
	subjects?: string[];
	grades?: PathInput[];
	rights?: RightsInput | null;
	inventories?: InventoryId[];
	elections?: ElectionInput[];
	tpt_base?: TptBaseInput;
}

export interface MappingView {
	inventory: InventoryId;
	mapping: string;
}

export interface CreatedProductView {
	product: string;
	mappings: MappingView[];
	/** How many already-answered elections were written. A question this
	 *  product had already settled writes no second answer, and the seller is
	 *  told rather than left to assume. */
	elections_recorded: number;
}

/** Every field is optional and an absent one is left alone. `body_format`
 *  travels only alongside the body it describes; the server refuses it on its
 *  own. */
export interface PatchProductBody {
	title?: string;
	body?: string;
	body_format?: CopyFormat;
	price?: PriceIntent;
	subjects?: string[];
	grades?: PathInput[];
	rights?: RightsInput;
	/** Given whole or not at all: the sidecar row is replaced rather than
	 *  merged, so a field the seller cleared reaches the server as cleared. */
	tpt_base?: TptBaseInput;
}

export interface PatchedProductView {
	product: string;
	/** The platforms this edit reaches when it is next synced. */
	reaches: InventoryId[];
}

export interface DeleteProductBody {
	/** The platforms whose listing is removed too. One removal job per entry,
	 *  on the same ledger every other write travels. */
	remove_from?: InventoryId[];
	/** Whether a bound listing this delete does not remove may be left
	 *  standing. Explicit and defaulted off. */
	leave_live?: boolean;
}

export interface RemovalView {
	inventory: InventoryId;
	job: string;
	mapping: string;
}

export interface DeletedProductView {
	product: string;
	removals: RemovalView[];
	left_live: InventoryId[];
}

export interface FileView {
	id: string;
	role: FileRole;
	kind: FileKind;
	byte_len: number;
	/** The digest of these bytes, which is how a handle names a file. Carried
	 *  so an edit form can seed a draft that is genuinely this product rather
	 *  than one whose files it can count and not name. */
	hash: string;
	scan: string;
	/** What the seller called this file, where a name was recorded. Absent for
	 *  a file stored before names existed and for a generated cover, and the
	 *  console renders the absence rather than substituting the kind. */
	name?: string;
}

/** What one file change did, and where it lands.
 *
 *  `reaches` names the marketplaces the change reaches on the next send, not
 *  the ones it has already reached: nothing on this path contacts a
 *  marketplace, so the copy already on one stands until that send. */
export interface AddedFileView {
	product: string;
	file: FileView;
	reaches: InventoryId[];
}

/** A replacement is a new row with a new identifier, so `removed` names the
 *  one that stopped being this resource's rather than leaving a client to
 *  assume its id survived. */
export interface ReplacedFileView {
	product: string;
	removed: string;
	file: FileView;
	/** The thumbnail this replacement redrew, absent where it did not. The
	 *  server renders it from the replacement's own bytes and reports what it
	 *  drew; a page that assumed would tell a seller their thumbnail changed
	 *  when it had not. */
	cover?: FileView;
	reaches: InventoryId[];
}

export interface RemovedFileView {
	product: string;
	file: string;
	/** What became of the thumbnail: left alone, redrawn from the file that
	 *  became the first one, or retired because nothing is left to draw one
	 *  from. Three states rather than an optional file, because the last is not
	 *  the absence of the second. */
	thumbnail:
		| { state: 'untouched' }
		| { state: 'redrawn'; file: FileView }
		| { state: 'retired' };
	reaches: InventoryId[];
}

export interface PathView {
	inventory: InventoryId;
	kind: TermKind;
	segments: string[];
	native_id?: string;
}

export interface AgeView {
	low_years: number;
	high_years: number;
}

export interface GradesView {
	/** `seller`, or `imported:<inventory>:<axis>`. Passed through rather than
	 *  parsed: the provenance is displayed, never branched on. */
	source: string;
	raw: PathView[];
	derived?: AgeView;
}

/** The TPT-base sidecar as the read returns it.
 *
 *  Every field is the name `TptBaseInput` sends, so the edit form seeds from
 *  this and patches with that without a second vocabulary in between. The two
 *  differ only in optionality: the read states each field, the write may omit
 *  one. */
export interface TptBaseView {
	thumbnail_mode: number;
	thumbnail_hashes: string[];
	video_preview_hash: string | null;
	additional_licence_minor_units: number | null;
	bundle_discount_minor_units: number | null;
	tax_code_id: number | null;
	subject_areas: string[];
	tags: string[];
	formats: string[];
	custom_categories: string[];
	appropriate_for_country: boolean | null;
	standards: { framework: number; code: string; tpt_node_id: number | null }[];
	teaching_duration_id: number | null;
	pages_or_slides: number | null;
	answer_key_id: number | null;
	copyright_declaration_id: number | null;
	status_user: number;
}

/** The product aggregate. `files` is payload, cover and previews alike, so a
 *  consumer filters on `role` rather than assuming the array is one kind. */
export interface ProductView {
	id: string;
	title: string;
	body: string;
	body_format: CopyFormat;
	price: unknown;
	files: FileView[];
	subjects: string[];
	grades: GradesView;
	rights?: PathView;
	/** Absent for a product with no sidecar row: one authored before the table
	 *  existed, or imported from a marketplace. Not the same as a row whose
	 *  fields are all unanswered. */
	tpt_base?: TptBaseView;
	created_at: number;
	updated_at: number;
}

// --------------------------------------------------------------- vocabulary

export interface CapView {
	limit: number;
	unit: LengthUnit;
}

/** `cap` absent means unmeasured, never unlimited. `required` is true only
 *  where a refusal is measured or documented; false records no such finding,
 *  which is not evidence the field is optional. */
export interface CanonicalFieldView {
	field: string;
	cap?: CapView;
	required: boolean;
}

export interface DelegationView {
	kind: Delegation;
	reason?: NonDelegableReason;
}

/** One admissible value of a native field: the token the adapter posts and the
 *  words a seller reads.
 *
 *  `label` is the id itself wherever the committed capture carries no words for
 *  it, so a value that is already its own name renders unchanged and no reading
 *  is invented on either side of the wire. */
export interface NativeValueView {
	id: string;
	label: string;
}

/** `values` is present only for a `closed` vocabulary. A `closed_uncaptured`
 *  one is closed and unheld, so a form must render it as a value the seller
 *  supplies at their own risk rather than as a select with no options. */
export interface NativeFieldView {
	name: string;
	direction: NativeDirection;
	required: boolean;
	vocabulary: NativeVocabularyKind;
	values?: NativeValueView[];
	delegation: DelegationView;
}

export interface AxisView {
	axis: TermKind;
	/** The native field this axis lands in, which is the key into `natives`. */
	native: string;
	cardinality: Cardinality;
	cap?: number;
	delegation: DelegationView;
	required: boolean;
}

/** Which licence values a create may carry, split by the branch the platform
 *  gates the write on. */
export interface LicenceGateView {
	native: string;
	free: string[];
	paid: string[];
}

export interface AttestationView {
	native: string;
	held_per_connection: boolean;
}

/** The write-side facts a form needs that the field registry does not record. */
export interface AuthoringView {
	payload_files: PayloadFileRule;
	body_wire: BodyWire;
	body_formats: CopyFormat[];
	price_floor_minor_units?: number;
	licence?: LicenceGateView;
	attestation?: AttestationView;
}

/** One marketplace's authoring vocabulary, served as data so the create form
 *  is driven by the registry rather than by a second copy of it here.
 *
 *  An axis in `absent_axes` is a disclosed loss the form states up front; an
 *  axis merely missing from `axes` is unmeasured and blocks at projection
 *  instead. */
export interface VocabularyView {
	inventory: InventoryId;
	marketplace: Marketplace;
	canonical: CanonicalFieldView[];
	natives: NativeFieldView[];
	axes: AxisView[];
	absent_axes: TermKind[];
	authoring: AuthoringView;
}

// -------------------------------------------------- the canonical form

/** One member of a `data[TaxonomyTags][]` picker.
 *
 *  `seller_writable` is false only where the capture measured that TPT's own
 *  create form renders no control for the facet: the three grade roll-ups
 *  drive buyer-facing browse filters and no seller can pick one. They are
 *  served rather than omitted so the form can explain the absence. */
export interface FacetView {
	slug: string;
	label: string;
	parent?: string;
	seller_writable: boolean;
}

/** One option of a listbox, radio group or switch.
 *
 *  `id` is the wire value and the list is ordered by it. `menu_index` is where
 *  TPT's own menu puts the option, which for Answer Key is not the same thing:
 *  its positions 3 and 5 carry ids 4 and 3, so a client that derived an id
 *  from a position would write "Included with Rubric" as "Does Not Apply". */
export interface FormOption {
	id: string;
	label: string;
	menu_index?: number;
	code?: string;
}

/** A picker's measured limit. An absent field is unmeasured, never unlimited:
 *  `subject_areas` has none because a capture TPT accepted already exceeded
 *  the number its form states. */
export interface FormCaps {
	grades?: number;
	subject_areas?: number;
	tags?: number;
	formats?: number;
	thumbnails?: number;
}

export interface SlotView {
	label: string;
	max_size_bytes: number;
	file_extensions: string[];
}

export interface FormLimits {
	title_max_utf16_units: number;
	description_max_length: number;
	min_price_minor_units: number;
	additional_licence_percentage: number;
	free_resource_page_guidance: number;
	product_file: SlotView;
	preview: SlotView;
	video_preview: SlotView;
	thumbnail: SlotView;
}

/** The copyright group. `preselect` is always false and is served rather than
 *  assumed: TPT arrives with its first attestation ticked, and a default here
 *  would make the seller's legal statement ours. */
export interface CopyrightView {
	preamble: string;
	options: FormOption[];
	preselect: boolean;
}

/** The Categories group's last control.
 *
 *  `label` is the words TPT itself renders, which name the seller's own
 *  country. It is null for every seller today and the absence is measured
 *  rather than pending: TPT publishes no country list and nothing yet reads
 *  the connected account's country back, so there is no country to name. A
 *  client renders `generic_label` instead of a blank. */
export interface LocalisationView {
	label: string | null;
	generic_label: string;
}

export interface FrameworkView {
	jurisdiction_id: number;
	name: string;
	button_label: string;
}

/** Every controlled list the canonical create form renders. */
export interface FormVocabularyView {
	grades: FacetView[];
	grade_columns: number[];
	subject_areas: FacetView[];
	tags: FacetView[];
	formats: FacetView[];
	tax_codes: FormOption[];
	teaching_durations: FormOption[];
	answer_keys: FormOption[];
	thumbnail_modes: FormOption[];
	copyright: CopyrightView;
	localisation: LocalisationView;
	statuses: FormOption[];
	standards_frameworks: FrameworkView[];
	caps: FormCaps;
	limits: FormLimits;
}

/** The draft as the form holds it, sent to the server for the verdict. */
export interface DraftInput {
	name?: string;
	/** Whether this draft is bound for a marketplace (D32). A resource kept on
	 *  Teachouse may carry no file; a marketplace listing may not, so the
	 *  payload rule is decided by the destination and the model cannot know it.
	 *  Optional and defaulting to false on the server, which is what lets a
	 *  saved template — a partial draft nobody has chosen a destination for —
	 *  round-trip without being held to a marketplace's rules. */
	for_marketplace?: boolean;
	payload_hash?: string | null;
	preview_hash?: string | null;
	video_preview_hash?: string | null;
	thumbnail_mode?: number | null;
	thumbnail_hashes?: string[];
	description?: string;
	free?: boolean;
	price_minor_units?: number | null;
	additional_licence_minor_units?: number | null;
	bundle_discount_minor_units?: number | null;
	tax_code_id?: number | null;
	grades?: string[];
	subject_areas?: string[];
	tags?: string[];
	formats?: string[];
	custom_categories?: string[];
	/** `data[ItemsLocalization][country_id_flag]`. Absent or null states nothing
	 *  about the control rather than answering it: the sidecar holds three
	 *  states, and a form that rendered the checkbox sends the box's own state
	 *  either way, so false arrives only as an answer. `null` travels because an
	 *  edit form seeded from a product that never answered has to be able to
	 *  leave it unanswered; `Option<bool>` on the server reads it as `None`. */
	appropriate_for_country?: boolean | null;
	standards?: { framework: number; code: string; tpt_node_id?: number | null }[];
	teaching_duration_id?: number | null;
	pages_or_slides?: number | null;
	answer_key_id?: number | null;
	copyright_declaration_id?: number | null;
	status_user?: number | null;
}

export interface RefusalView {
	group: FormGroup;
	control?: string;
	message: string;
}

export interface AdvisoryView {
	group: FormGroup;
	message: string;
}

/** What the server says the form must refuse. The client runs the same rules
 *  as the seller types; this is the authority. */
export interface CheckView {
	submittable: boolean;
	refusals: RefusalView[];
	advisories: AdvisoryView[];
}

export interface StandardView {
	/** The framework this standard is actually from, read off the match rather
	 *  than echoed from the request, so a result never misstates where it came
	 *  from. */
	framework: number;
	code: string;
	statement: string;
	/** The subject its mirrored set carries, so a code never renders bare. */
	subject?: string;
	/** The grades the mirrored set covers, in the words a teacher uses.
	 *
	 *  Rendered by the server rather than derived here, so one set reads the
	 *  same wherever it appears. Absent where the set records no grades at all,
	 *  which the picker shows as no grade rather than as an empty one. */
	grade_band?: string;
	/** The mirror's own identifier, and the only unique field on this view.
	 *
	 *  A code is not unique: 814 TEKS codes name more than one addressable node
	 *  with a different statement, `1.1.A` four times across four subjects, and
	 *  all of them arrive as separate items. Key a list on this, never on the
	 *  code, or reconciliation can attach one standard's row to another's. */
	source_guid: string;
	/** Absent for a standard the server can display and cannot yet post: the id
	 *  is TPT's own search-index identifier, and one no current capture vouches
	 *  for is withheld rather than guessed. */
	tpt_node_id?: number;
}

/** One notice a framework's licence obliges a display to carry, and where.
 *
 *  `placement` is `wherever_displayed`, `site_footer_and_every_page_using_the_mark`
 *  or `with_the_data`. Several rather than one string, because the obligations
 *  differ in where they must appear and flattening them would discard the half
 *  a licence turns on. */
export interface NoticeView {
	text: string;
	placement: string;
}

/** `not_ingested` is a state rather than an empty result: no standard matched
 *  your words, against no standard exists here yet. The notices travel with the
 *  results because the licence obliges them wherever a standard is shown, and
 *  they are the server's to state rather than the client's to remember. */
export interface StandardsSearchView {
	state: StandardsState;
	framework: number;
	items: StandardView[];
	notices: NoticeView[];
}

// ----------------------------------------------------------------- taxonomy

/** One canonical term: the relation above the marketplaces' own field tables,
 *  which the projection carries into whichever platform a product is authored
 *  for.
 *
 *  `id` is the identifier `CreateProductBody.subjects` carries. `parent` is the
 *  broader term this one sits under — a topic names its subject — and is absent
 *  for a root, which every subject is. */
export interface TermView {
	id: string;
	kind: TermKind;
	label: string;
	parent?: string;
}

export interface TermsView {
	terms: TermView[];
}

/** Bytes to `POST /v1/uploads`, with the fraction sent reported as it goes.
 *
 *  XMLHttpRequest rather than `fetch` for one reason: a payload file is up to
 *  256 MiB and `fetch` reports no upload progress, so a seller watching a
 *  large ZIP would see nothing at all until it landed. The body is the raw
 *  `File` either way — not a multipart part — because the handler reads the
 *  whole body as bytes and a multipart wrapper would be prefix noise it would
 *  then ingest as the file. */
export function upload(
	file: File,
	archive: ArchiveMode,
	onProgress?: (fraction: number) => void
): Promise<UploadedView> {
	return new Promise((resolve, reject) => {
		const request = new XMLHttpRequest();
		request.open('POST', `/v1/uploads?archive=${archive}`);
		request.setRequestHeader('accept', 'application/json');
		request.upload.addEventListener('progress', (event) => {
			if (onProgress && event.lengthComputable && event.total > 0) {
				onProgress(event.loaded / event.total);
			}
		});
		request.addEventListener('load', () => {
			let body: unknown = null;
			try {
				body = JSON.parse(request.responseText);
			} catch {
				body = null;
			}
			if (request.status >= 200 && request.status < 300) {
				resolve(body as UploadedView);
				return;
			}
			reject(new ApiFailure(request.status, body as APIErrorBody | null));
		});
		// A transport failure carries no status of its own; 0 is the one this
		// client already reads as "the request never reached a server".
		request.addEventListener('error', () => reject(new ApiFailure(0, null)));
		request.addEventListener('abort', () => reject(new ApiFailure(0, null)));
		request.send(file);
	});
}

// --------------------------------------------------------------- endpoints

export const api = {
	whoami: () => request<Whoami>('/v1/whoami'),
	exchange: (token: string) => post<Whoami>('/v1/session', { token }),
	logout: () => request<void>('/v1/session', { method: 'DELETE' }),

	org: () => request<OrgView>('/v1/org'),

	/** The rename and the claim are one route: a claim is the first slug write
	 *  and a second is a rename. At least one field must be present; a body
	 *  naming neither is refused 422 rather than treated as a no-op. The answer
	 *  is the stored row, because a body naming one field leaves the other
	 *  alone and only the stored row states both. */
	updateOrg: (change: { name?: string; slug?: string }) => patch<OrgView>('/v1/org', change),

	/** Whether a slug is free, as the claim screen asks while the seller types.
	 *  Session-gated and bounded per session, because the verdict is otherwise a
	 *  directory of every tenant's slug read one guess at a time. A malformed or
	 *  reserved slug answers 422 rather than `available: false`. */
	slugAvailability: (slug: string) =>
		request<SlugAvailability>(`/v1/org/slug/${encodeURIComponent(slug)}`),

	/** One keyset page of the catalogue, optionally narrowed to the items
	 *  carrying one label. The label narrows the query rather than the page, so
	 *  paging a filtered catalogue is the same walk as paging the whole one. */
	products: (cursor?: string | null, label?: string | null) => {
		const query = new URLSearchParams();
		if (cursor) {
			query.set('cursor', cursor);
		}
		if (label) {
			query.set('label', label);
		}
		const suffix = query.size === 0 ? '' : `?${query.toString()}`;
		return request<ProductsPage>(`/v1/products${suffix}`);
	},
	product: (id: string) => request<ProductView>(`/v1/products/${id}`),
	mappings: () => request<{ mappings: MappingHead[] }>('/v1/mappings'),
	/** Every label this organisation uses, which is what the board's filter
	 *  lists. A label nothing carries is not in the set: it left the
	 *  vocabulary when the last item stopped carrying it. */
	labels: () => request<LabelsView>('/v1/labels'),

	/** The calling organisation's own projection overrides. Another
	 *  organisation's are unreachable: the server pins the tenant itself. */
	overrides: () => request<OverridesView>('/v1/mappings/overrides'),
	/** Record one override, replacing any this organisation already holds for
	 *  the same marketplace, axis and term. Answers no content. */
	setOverride: (body: OverrideInput) => post<void>('/v1/mappings/overrides', body),
	/** Withdraw one override, leaving the global relation to answer again. A
	 *  withdrawal that matches nothing succeeds: the state it asks for is the
	 *  state that already holds. */
	withdrawOverride: (body: WithdrawOverrideInput) =>
		request<void>('/v1/mappings/overrides', {
			method: 'DELETE',
			headers: { 'content-type': 'application/json' },
			body: JSON.stringify(body)
		}),
	/** The labels on one item. */
	productLabels: (product: string) =>
		request<LabelsView>(`/v1/products/${product}/labels`),
	/** Replace the labels on one item. The whole set, not a delta: a delta
	 *  would leave removing the last label with no spelling. */
	setProductLabels: (product: string, labels: string[]) =>
		put<LabelsView>(`/v1/products/${product}/labels`, { labels }),
	/** Add a marketplace to an item that already exists. The mapping comes
	 *  back unbound, exactly as a create's own does, so a send afterwards is
	 *  the ordinary job path; nothing here contacts the marketplace. An item
	 *  that already reaches it is refused with `mapping_already_exists`. */
	addMapping: (product: string, inventory: InventoryId) =>
		post<MappingHead>(`/v1/products/${product}/mappings`, { inventory }),
	/** Bind a mapping to a listing the console did not create, by its page URL.
	 *  Writes the catalogue only: no marketplace is contacted, so the binding
	 *  starts unverified and the engine's own read-back is what confirms the
	 *  listing exists and is the seller's. Refuses a link for another
	 *  marketplace, a link that is not a listing page, a mapping that already
	 *  binds one, and a listing another of the seller's items already claims. */
	bindMapping: (mapping: string, listingUrl: string) =>
		post<MappingHead>(`/v1/mappings/${mapping}/bind`, { listing_url: listingUrl }),
	analytics: () => request<AnalyticsSummary>('/v1/analytics/summary'),

	/** One marketplace's authoring vocabulary. Cached per inventory: it is
	 *  the registry rendered onto the wire and changes only when the server
	 *  does. */
	vocabulary: (inventory: InventoryId) =>
		request<VocabularyView>(`/v1/vocabulary/${inventory}`),
	/** The canonical terms of one kind, ordered by the words they read. Cached
	 *  like a vocabulary: the taxonomy is ours rather than an organisation's and
	 *  changes only when the server does. */
	terms: (kind: TermKind) => request<TermsView>(`/v1/taxonomy/terms?kind=${kind}`),

	/** Every controlled list the canonical create form renders. One read for
	 *  the whole form, cached like a vocabulary: it is the committed TPT
	 *  capture rendered onto the wire and changes only when the server does. */
	formVocabulary: () => request<FormVocabularyView>('/v1/authoring/vocabulary'),
	/** The server's verdict on a draft. The form runs the same rules inline so
	 *  a seller sees a message as they type; this is what decides. */
	checkDraft: (draft: DraftInput) => post<CheckView>('/v1/authoring/check', draft),
	/** One jurisdiction's standards, or the honest not-ingested state. */
	standardsSearch: (framework: number, query: string) =>
		request<StandardsSearchView>(
			`/v1/standards/search?framework=${framework}&q=${encodeURIComponent(query)}`
		),
	upload,
	createProduct: (body: CreateProductBody) =>
		post<CreatedProductView>('/v1/products', body),
	patchProduct: (id: string, body: PatchProductBody) =>
		patch<PatchedProductView>(`/v1/products/${id}`, body),
	/** Add one file to a resource that already exists, naming a handle
	 *  `POST /v1/uploads` returned. A catalogue write and nothing else. */
	addProductFile: (product: string, role: FileRole, handle: FileHandle) =>
		post<AddedFileView>(`/v1/products/${product}/files`, { role, handle }),
	/** Swap one file's bytes. One argument and deliberately no others: the
	 *  replacement keeps the role of the file it replaces, and the thumbnail
	 *  the server may redraw is rendered there from these bytes rather than
	 *  named here, so there is no handle a client could point at a file of its
	 *  choosing. `ReplacedFileView.cover` reports what it drew. */
	replaceProductFile: (product: string, file: string, handle: FileHandle) =>
		put<ReplacedFileView>(`/v1/products/${product}/files/${file}`, { handle }),
	/** Retire one file. Refused with `payload_missing` where it is the only
	 *  payload file the resource has: a resource keeps at least one. */
	removeProductFile: (product: string, file: string) =>
		request<RemovedFileView>(`/v1/products/${product}/files/${file}`, { method: 'DELETE' }),

	/** The body is mandatory in practice even though the server defaults it:
	 *  a delete that removes nothing remotely has to say so, and `leave_live`
	 *  is the only way to say it. */
	deleteProduct: (id: string, body: DeleteProductBody) =>
		request<DeletedProductView>(`/v1/products/${id}`, {
			method: 'DELETE',
			headers: { 'content-type': 'application/json' },
			body: JSON.stringify(body)
		}),

	/** `intent` is where the seller asked the listing to end up. Absent keeps
	 *  the endpoint's own default, which is a draft. */
	createJob: (
		inventory: InventoryId,
		mappings: string[],
		idempotencyKey: string,
		intent?: PublishIntent
	) =>
		post<CreatedJob>(
			'/v1/jobs',
			{ inventory, mappings, intent },
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

	/** Ask for a sync or a migrate. The key is the request's own identity on the
	 *  server rather than a deduplication token beside it, so a retried submit is
	 *  the same request, and the key is what the request is read back by. */
	createSyncRequest: (body: SyncRequestBody, idempotencyKey: string) =>
		post<SyncRequestAck>('/v1/sync', body, { 'idempotency-key': idempotencyKey }),
	/** One request as it fills. This is what the console watches between the
	 *  submit and the ledger having a run to show. */
	syncRequest: (id: string) => request<SyncRequestView>(`/v1/sync/${id}`),
	/** Every request this organisation has made, newest first and bounded by
	 *  the server. It is what makes a request created on one machine reachable
	 *  from another and after a restart: a device-branch migrate mints no job
	 *  while it is pending, so until it does there is nothing else in the
	 *  console that names it. */
	syncRequests: () => request<SyncRequestsView>('/v1/sync'),

	/** The seller's own machines. Registration and heartbeat are the desktop
	 *  client's calls, not the console's, so they are deliberately absent
	 *  here. */
	devices: () => request<DevicesView>('/v1/devices'),
	/** Sign one machine out. The device wipes its marketplace sessions on its
	 *  next check-in; one that never reconnects keeps them until the
	 *  marketplace expires them, which the page states. */
	revokeDevice: (device: string) => post<DeviceView>(`/v1/devices/${device}/revoke`, {}),

	/** Every marketplace connection this organisation holds. The envelope the
	 *  route answers with is opened here, so no caller can hold a second
	 *  shape of the same read. */
	connections: () => request<ConnectionsView>('/v1/connections').then(connectionList),
	/** Declare who holds the copyright in the work sent to one marketplace.
	 *
	 *  The path segment is the marketplace's own serde name — `Tpt`, `Tes`,
	 *  `Etsy` — which is the spelling the generated vocabulary and the device
	 *  heartbeat already use; anything else is a 404. Declaring again replaces
	 *  what stands, and the answer is the declaration as stored rather than as
	 *  sent. A marketplace whose automation runs server-side under our own
	 *  token refuses with 422: no device composes a write for it and nothing
	 *  would read the declaration back. */
	declareAuthorship: (marketplace: Marketplace, name: string) =>
		post<{ marketplace: Marketplace; name: string; attested_at: number }>(
			`/v1/connections/${marketplace}/authorship`,
			{ name }
		),

	/** Terminal, and nothing undoes it. The device check-in lifts a connection
	 *  to `linked` only from `unlinked`, `linking` or `needs_reauth`, so a
	 *  revoked marketplace stays revoked for that tenant however many live
	 *  sessions their machines report. The operator and security path; a
	 *  seller's Disconnect is `disconnect` below. */
	revoke: (connection: string) =>
		post<{ connections: number; elapsed_ms: number }>(
			`/v1/connections/${connection}/revoke`,
			{}
		),
	/** The seller's own disconnect: reversible, because the next check-in from
	 *  a machine holding that login lifts the row back. Answers the same two
	 *  numbers `revoke` does — one row moved or none — and answers zero rather
	 *  than a not-found for a connection this tenant does not have. */
	disconnect: (connection: string) =>
		post<{ connections: number; elapsed_ms: number }>(
			`/v1/connections/${connection}/disconnect`,
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
	adminImportDrain: () => request<ImportDrainView>('/v1/admin/import-drain'),
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
