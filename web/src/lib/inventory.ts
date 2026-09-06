// Where one item stands on one marketplace, and what the seller can do
// about it. This is the inventory board's whole vocabulary: one chip per
// marketplace per row, its words, its tone and the one action it admits.
// Pure, so it tests without a component.
//
// Every state below is derived from a field the API already serves. Nothing
// here interpolates a version, a freshness or a marketplace-side fact: a
// published-but-out-of-date state would need a file version nothing records,
// so it is absent rather than guessed.

import type {
	ConnectionView,
	InventoryStatus,
	ItemView,
	MappingHead,
	ProductHead
} from '$lib/api';
import { gateLabel } from '$lib/gates';
import { INVENTORY_ORDER, MARKETPLACE_OF } from '$lib/listings-view';
import { AUTHORABLE_PLATFORMS, marketplacesHref } from '$lib/platforms';
import { standingOf } from '$lib/tes-portfolio';
import type {
	BlockedGate,
	InventoryId,
	Marketplace,
	TransportClass
} from '$lib/generated/vocab';

/** Which branch of decision D1 each marketplace's automation runs on.
 *
 * A total map mirroring `tam_types::Marketplace::transport_class`, because no
 * view serves the class: a marketplace added in Rust stops this file
 * type-checking rather than rendering under the wrong branch. Serving it on
 * the vocabulary would remove the mirror. */
export const TRANSPORT_OF: Record<Marketplace, TransportClass> = {
	Tes: 'SellerDevice',
	Tpt: 'SellerDevice',
	Etsy: 'OfficialApi'
};

/** Whether this marketplace's work originates on the seller's own machine.
 *
 * The honest half of D1, and the reason a row can say "runs while your device
 * is on" for one platform and not for another. */
export function onSellerDevice(inventory: InventoryId): boolean {
	return TRANSPORT_OF[MARKETPLACE_OF[inventory]] === 'SellerDevice';
}

/** Which gates are the seller signing in again, rather than anything else.
 *
 * A total map over the generated union, so a gate added in Rust is classified
 * here or fails the web lane; a sign-in gate reads as an action the seller
 * takes on their own device, and every other gate reads as a wait. */
const SIGN_IN_GATE: Record<BlockedGate, boolean> = {
	reconciliation: false,
	election: false,
	currency_unknown: false,
	currency_mismatch: false,
	cover_missing: false,
	scan_incomplete: false,
	awaiting_counterpart: false,
	binding: false,
	unbound: false,
	subject_diverged: false,
	lifecycle_diverged: false,
	ReauthRequired: true,
	awaiting_seller_signin: true,
	// Nothing the seller does clears this one: the write went out and we are
	// going back to look for its answer.
	awaiting_marketplace_answer: false
};

/** Where the seller goes to clear each gate.
 *
 * `election` points at the run rather than at a screen because there is none:
 * `GET /v1/elections/items` is served and no client reads it, so the console
 * can show that an answer is wanted and cannot yet ask for one. */
const GATE_DESTINATION: Record<BlockedGate, 'queue' | 'run' | 'connections' | 'listing'> = {
	reconciliation: 'queue',
	election: 'run',
	currency_unknown: 'listing',
	currency_mismatch: 'listing',
	cover_missing: 'listing',
	scan_incomplete: 'run',
	awaiting_counterpart: 'run',
	binding: 'run',
	unbound: 'run',
	subject_diverged: 'run',
	lifecycle_diverged: 'run',
	ReauthRequired: 'connections',
	awaiting_seller_signin: 'connections',
	// The run, as `election` does, because no screen clears this either: the
	// console shows that something is awaited without pretending to offer an
	// action.
	awaiting_marketplace_answer: 'run'
};

function signInGate(gate: string | undefined): boolean {
	return gate !== undefined && SIGN_IN_GATE[gate as BlockedGate] === true;
}

/** The gate in the seller's words, or the honest absence of one. A blocked
 *  item always carries a gate; a parked one may not, and inventing a cause
 *  would be worse than naming that we do not hold it. */
function waitingOn(gate: string | undefined): string {
	return gate === undefined ? 'a check we have not been told the name of' : gateLabel(gate);
}

/** How many of the newest runs the inventory reads items from.
 *
 * Bounded rather than paginated, and the reason is gap G2: a job item is
 * reachable only through the job that holds it, so answering "what is
 * happening to this listing here" costs one read per run. A mapping whose
 * newest item is older than this window falls back to its stored standing,
 * which is the honest answer rather than a stale one. */
export const WORK_RUNS = 8;

export type MarketplaceState =
	| 'not_listed'
	| 'draft'
	| 'listed'
	| 'in_flight'
	| 'blocked'
	| 'needs_signin'
	| 'stranded'
	| 'failed';

/** The chip's own word. Short enough to sit inside a strip of five and still
 *  be read at a glance, which is the whole point of the strip. */
export const STATE_LABEL: Record<MarketplaceState, string> = {
	not_listed: 'Not listed',
	draft: 'Draft',
	listed: 'Listed',
	in_flight: 'Sending',
	blocked: 'Blocked',
	needs_signin: 'Sign in',
	stranded: 'Held',
	failed: 'Failed'
};

export type ChipTone = 'ok' | 'run' | 'bad' | 'mut';

export const STATE_TONE: Record<MarketplaceState, ChipTone> = {
	not_listed: 'mut',
	draft: 'mut',
	listed: 'ok',
	in_flight: 'run',
	blocked: 'run',
	needs_signin: 'bad',
	stranded: 'bad',
	failed: 'bad'
};

/** The states a person has to do something about, in the order they cost the
 *  seller. This is the research's "needs attention" column, computed rather
 *  than stored. */
export const ATTENTION_STATES: readonly MarketplaceState[] = [
	'needs_signin',
	'stranded',
	'failed',
	'blocked'
];

export interface ChipAction {
	label: string;
	href: string;
	/** Whether the href leaves this console. Set only where it does, so a
	 *  renderer knows to open a new tab and withhold the referrer; every
	 *  in-app route omits it. */
	external?: boolean;
}

export interface MarketplaceChip {
	inventory: InventoryId;
	state: MarketplaceState;
	label: string;
	tone: ChipTone;
	/** One sentence naming the cause, in the seller's words. */
	detail: string;
	/** The single action this state admits, or null where the console has no
	 *  endpoint to offer one. */
	action: ChipAction | null;
	/** Whether this marketplace's work runs on the seller's own machine. */
	onDevice: boolean;
	/** Why sending is paused for the whole marketplace, where it is. An
	 *  overlay rather than a state: the listing still stands where it stood. */
	paused: string | null;
}

/** One job item, carrying the run that holds it, because an item names its
 *  mapping and not the job it belongs to. */
export interface WorkItem {
	job: string;
	item: ItemView;
}

/** The newest item touching each mapping.
 *
 * Newest rather than every one, because a chip states what is happening now:
 * a failure a later run replaced is history, and the run pages hold it. */
export function newestWork(work: readonly WorkItem[]): Map<string, WorkItem> {
	const newest = new Map<string, WorkItem>();
	for (const entry of work) {
		const held = newest.get(entry.item.mapping);
		if (held === undefined || entry.item.created_at > held.item.created_at) {
			newest.set(entry.item.mapping, entry);
		}
	}
	return newest;
}

export interface ChipInput {
	product: string;
	inventory: InventoryId;
	mapping: MappingHead | undefined;
	work: WorkItem | undefined;
	connection: ConnectionView | undefined;
	status: InventoryStatus | undefined;
}

interface Verdict {
	state: MarketplaceState;
	detail: string;
	action: ChipAction | null;
}

function gateAction(
	gate: string | undefined,
	inventory: InventoryId,
	product: string,
	job: string
): ChipAction {
	const run: ChipAction = { label: 'Open the run', href: `/sync/${job}` };
	if (gate === undefined) {
		return run;
	}
	switch (GATE_DESTINATION[gate as BlockedGate]) {
		case 'queue':
			return { label: 'Answer in Reconciliation', href: '/reconciliation' };
		case 'connections':
			return { label: 'Open Marketplaces', href: marketplacesHref(inventory) };
		case 'listing':
			return { label: 'Open the item', href: `/resources/${product}` };
		default:
			return run;
	}
}

/** What the newest run says about this mapping, or null where it says nothing
 *  the standing does not already say.
 *
 * A settled run that succeeded returns null on purpose: the mapping's own two
 * states are then the better answer, and repeating the run's verdict beside
 * them would be two sentences for one fact. */
function ofWork(input: ChipInput, entry: WorkItem): Verdict | null {
	const gate = entry.item.blocked_on;
	const run: ChipAction = { label: 'Open the run', href: `/sync/${entry.job}` };
	switch (entry.item.state) {
		case 'queued':
		case 'leased':
		case 'running':
		case 'verifying':
			return {
				state: 'in_flight',
				detail: onSellerDevice(input.inventory)
					? 'A send is under way, and it runs while your own device is on.'
					: 'A send is under way.',
				action: run
			};
		case 'parked_live':
		case 'parked_cold':
			return {
				state: 'stranded',
				// One gate on this branch is not an interruption: the write
				// completed and the answer is what is missing, so it gets its
				// own sentence rather than the one about being held rather than
				// retried blind. Every other cause keeps that sentence.
				detail:
					gate === 'awaiting_marketplace_answer'
						? 'We sent the listing, did not get an answer we could trust, and are going back to look for it.'
						: `A send reached this marketplace and was interrupted, so it is held rather than retried blind. It is waiting on ${waitingOn(gate)}.`,
				action: signInGate(gate)
					? { label: 'Open Marketplaces', href: marketplacesHref(input.inventory) }
					: gateAction(gate, input.inventory, input.product, entry.job)
			};
		case 'blocked':
			if (signInGate(gate)) {
				return {
					state: 'needs_signin',
					detail: onSellerDevice(input.inventory)
						? 'This is waiting on you signing in to the marketplace on your own device.'
						: 'This is waiting on you signing in to the marketplace again.',
					action: { label: 'Open Marketplaces', href: marketplacesHref(input.inventory) }
				};
			}
			return {
				state: 'blocked',
				detail: `Nothing is being sent while this waits on ${waitingOn(gate)}.`,
				action: gateAction(gate, input.inventory, input.product, entry.job)
			};
		case 'settled':
			if (entry.item.outcome !== 'failed') {
				return null;
			}
			return {
				state: 'failed',
				detail:
					entry.item.failure_detail ??
					'The last send to this marketplace failed. The run keeps its own reason.',
				action: run
			};
		default: {
			// A state this bundle has no word for. The `never` binding is the
			// compile-time half: a state added in Rust widens `ItemState` and
			// stops this assignment compiling until the switch above names it.
			// The verdict is the run-time half, and it is needed because the
			// console is a static bundle served from the control plane: a
			// deploy that widens the enum does not rebuild the page a seller
			// already has open. Without it the switch falls through as
			// `undefined`, which the `!== null` test at the call site lets
			// past, and one item on one mapping takes the whole board down.
			const unnamed: never = entry.item.state;
			return {
				state: 'in_flight',
				detail:
					'This run is in a state this app does not have a word for yet. Open the run to see what it says.',
				action: run
			};
		}
	}
}

/** What the mapping's own two stored states say. */
function ofMapping(input: ChipInput, mapping: MappingHead): Verdict {
	// Bound rather than switched on directly, so the arm below can narrow it.
	const standing = standingOf(mapping);
	switch (standing) {
		case 'live':
			return {
				state: 'listed',
				detail: 'This marketplace is showing the listing.',
				// The listing's own page is what a seller wants from a listed
				// chip, and it is the affordance Vendoo's strip is most used
				// for. Falls back to the item where the server serves no URL:
				// a marketplace whose page shape it has not observed.
				action:
					mapping.listing_url === null
						? { label: 'Open the item', href: `/resources/${input.product}` }
						: { label: 'Open the listing', href: mapping.listing_url, external: true }
			};
		case 'draft':
			return {
				state: 'draft',
				detail: 'This marketplace holds the listing and is not showing it to buyers yet.',
				action: { label: 'Open the item', href: `/resources/${input.product}` }
			};
		case 'unsent':
			return {
				state: 'not_listed',
				detail: 'Chosen for this item and never sent.',
				action: { label: 'Open the item', href: `/resources/${input.product}` }
			};
		case 'other':
			return {
				state: 'in_flight',
				detail:
					'A send is out and we have not recorded what came of it. Nothing else is sent until it settles.',
				action: { label: 'Open the item', href: `/resources/${input.product}` }
			};
		default: {
			// `standingOf` is total over its four answers today, so this arm is
			// unreachable. It is here because this function and `ofWork` are
			// read as a pair, and a reader should not have to walk into
			// `tes-portfolio.ts` to prove that only one of them can fall
			// through. The `never` binding fails the build if that stops being
			// true.
			const unnamed: never = standing;
			return {
				state: 'in_flight',
				detail:
					'This listing is in a state this app does not have a word for yet. Open the item to see what it says.',
				action: { label: 'Open the item', href: `/resources/${input.product}` }
			};
		}
	}
}

/**
 * One chip: where this item stands on this marketplace.
 *
 * The precedence is deliberate. What the newest run says outranks the stored
 * standing, because the standing is what was true and the run is what is
 * happening; a dropped connection outranks both, because Vendoo's own reviews
 * name the silent connection drop as the failure that costs sellers most, and
 * a listing standing behind a dead connection is exactly that silence. The
 * detail line still names the standing, so nothing is hidden by the ordering.
 */
export function chipFor(input: ChipInput): MarketplaceChip {
	const paused =
		input.status?.halted === true
			? (input.status.reason ??
				'Sending is paused for this marketplace. Nothing is lost; queued work waits.')
			: null;
	const onDevice = onSellerDevice(input.inventory);
	// The parameter admits `undefined` only so the refusal below can be
	// written. A verdict function that falls through its switch returns
	// `undefined` through a `Verdict | null` signature and the `!== null` test
	// at the call site lets it past; reading `.state` off it then fails two
	// frames from the function that was actually wrong, with a message naming
	// neither it nor the value. Both verdict functions are total now, so this
	// is for the next one.
	const dressed = (verdict: Verdict | undefined): MarketplaceChip => {
		if (verdict === undefined) {
			throw new TypeError(
				`no verdict for ${input.inventory}: a verdict function fell through its switch`
			);
		}
		return {
			inventory: input.inventory,
			state: verdict.state,
			label: STATE_LABEL[verdict.state],
			tone: STATE_TONE[verdict.state],
			detail: verdict.detail,
			action: verdict.action,
			onDevice,
			paused
		};
	};

	if (input.mapping === undefined) {
		return dressed({
			state: 'not_listed',
			detail:
				'This marketplace has never seen this item, and marketplaces are chosen when the draft is created.',
			action: null
		});
	}

	const fromWork = input.work === undefined ? null : ofWork(input, input.work);
	if (fromWork !== null) {
		return dressed(fromWork);
	}

	const standing = ofMapping(input, input.mapping);
	if (input.connection === undefined) {
		return dressed({
			state: 'needs_signin',
			detail: `No account is linked for this marketplace yet.${
				standing.state === 'listed' ? ' The listing itself is still up.' : ''
			}`,
			action: { label: 'Open Marketplaces', href: marketplacesHref(input.inventory) }
		});
	}
	if (input.connection.status === 'disconnected') {
		return dressed({
			state: 'needs_signin',
			detail: onDevice
				? `Nothing usable is stored for this marketplace; sign in again on your own device.${
						standing.state === 'listed' ? ' The listing itself is still up.' : ''
					}`
				: 'Nothing usable is stored for this marketplace; re-link it to let queued work continue.',
			action: { label: 'Open Marketplaces', href: marketplacesHref(input.inventory) }
		});
	}
	return dressed(standing);
}

export interface RowInput {
	product: ProductHead;
	mappings: readonly MappingHead[];
	work: ReadonlyMap<string, WorkItem>;
	connections: readonly ConnectionView[];
	statuses: readonly InventoryStatus[];
}

export interface InventoryRow {
	product: ProductHead;
	chips: MarketplaceChip[];
	/** The chips a person has to act on, worst first. */
	attention: MarketplaceChip[];
	/** The mappings this row could send, keyed by the marketplace they reach. */
	mapped: Map<InventoryId, MappingHead>;
}

/** Which marketplaces this row shows a chip for.
 *
 * Every marketplace this console can author for, so "not listed here" is
 * visible rather than absent, plus any this item is already mapped onto,
 * so a mapping made elsewhere never disappears from the strip. */
export function stripFor(mappings: readonly MappingHead[]): InventoryId[] {
	const shown = new Set<InventoryId>([
		...AUTHORABLE_PLATFORMS,
		...mappings.map((mapping) => mapping.inventory)
	]);
	return INVENTORY_ORDER.filter((inventory) => shown.has(inventory));
}

export function rowFor(input: RowInput): InventoryRow {
	const mapped = new Map(input.mappings.map((mapping) => [mapping.inventory, mapping]));
	const chips = stripFor(input.mappings).map((inventory) => {
		const mapping = mapped.get(inventory);
		return chipFor({
			product: input.product.id,
			inventory,
			mapping,
			work: mapping === undefined ? undefined : input.work.get(mapping.id),
			connection: input.connections.find(
				(connection) => connection.marketplace === MARKETPLACE_OF[inventory]
			),
			status: input.statuses.find((status) => status.inventory === inventory)
		});
	});
	const attention = ATTENTION_STATES.flatMap((state) =>
		chips.filter((chip) => chip.state === state)
	);
	return { product: input.product, chips, attention, mapped };
}

export type StandingFilter = 'all' | 'attention' | MarketplaceState;

export interface Filters {
	query: string;
	marketplace: InventoryId | 'all';
	standing: StandingFilter;
}

export const NO_FILTERS: Filters = { query: '', marketplace: 'all', standing: 'all' };

/** Whether a row survives the filter bar.
 *
 * The marketplace filter narrows which chips the standing filter reads, so
 * "TPT" and "needs attention" together mean rows needing attention on TPT
 * rather than rows on TPT that need attention anywhere. A marketplace on its
 * own means the rows carried there, because every row has a chip for every
 * marketplace this console authors for and matching on the chip alone would
 * narrow nothing; a marketplace with "not listed" is therefore the query that
 * finds what to cross-list next. */
export function matchesFilters(row: InventoryRow, filters: Filters): boolean {
	const needle = filters.query.trim().toLowerCase();
	if (needle.length > 0 && !row.product.title.toLowerCase().includes(needle)) {
		return false;
	}
	if (filters.standing === 'all') {
		return filters.marketplace === 'all' || row.mapped.has(filters.marketplace);
	}
	const chips =
		filters.marketplace === 'all'
			? row.chips
			: row.chips.filter((chip) => chip.inventory === filters.marketplace);
	if (filters.standing === 'attention') {
		return chips.some((chip) => ATTENTION_STATES.includes(chip.state));
	}
	return chips.some((chip) => chip.state === filters.standing);
}

export interface InventoryTally {
	/** Rows in the catalogue. */
	total: number;
	/** Rows the filters let through. */
	shown: number;
	/** Rows carrying at least one chip a person has to act on. */
	attention: number;
	/** Marketplaces carrying at least one listed chip. */
	listedOn: number;
}

export function inventoryTally(
	rows: readonly InventoryRow[],
	shown: readonly InventoryRow[]
): InventoryTally {
	const listedOn = new Set<InventoryId>();
	let attention = 0;
	for (const row of rows) {
		if (row.attention.length > 0) {
			attention += 1;
		}
		for (const chip of row.chips) {
			if (chip.state === 'listed') {
				listedOn.add(chip.inventory);
			}
		}
	}
	return { total: rows.length, shown: shown.length, attention, listedOn: listedOn.size };
}

/** The mappings a bulk send would address on one marketplace, and the items it
 *  would have to map onto that marketplace first.
 *
 * Both reported rather than one silently dropped: a seller who selected twelve
 * rows and sends to a marketplace nine of them carry needs to be told what
 * happens to the three, which is the failure mode Vendoo's bulk modal is
 * criticised for. They are now added rather than passed over, so the count the
 * dialog states is a count of items it will map. */
export interface BulkTarget {
	inventory: InventoryId;
	mappings: string[];
	/** The products with no mapping onto this marketplace, by identifier, in
	 *  the order the table shows them. */
	unmapped: string[];
}

export function bulkTarget(
	rows: readonly InventoryRow[],
	inventory: InventoryId
): BulkTarget {
	const mappings: string[] = [];
	const unmapped: string[] = [];
	for (const row of rows) {
		const mapping = row.mapped.get(inventory);
		if (mapping === undefined) {
			unmapped.push(row.product.id);
			continue;
		}
		mappings.push(mapping.id);
	}
	return { inventory, mappings, unmapped };
}

export interface RunRow {
	job: string;
	inventory: InventoryId;
	/** This listing's own part in that run, as the item recorded it. */
	state: ItemView['state'];
	outcome: ItemView['outcome'];
	created_at: number;
}

/**
 * The runs among those read that touched this listing, newest first.
 *
 * The item screen links each one into `/sync/{job}`, which already renders the
 * run's items, gates and events off the ledger; rebuilding that timeline here
 * would be a second reading of the same rows that could disagree with it.
 * Bounded by the same window the chips are, so a listing whose last send is
 * older than the window shows none rather than a partial history, and the sync
 * pages hold the whole record.
 */
export function runsFor(
	mappings: readonly MappingHead[],
	work: ReadonlyMap<string, WorkItem>
): RunRow[] {
	const rows = new Map<string, RunRow>();
	for (const mapping of mappings) {
		const entry = work.get(mapping.id);
		if (entry === undefined) {
			continue;
		}
		const held = rows.get(entry.job);
		if (held !== undefined && held.created_at >= entry.item.created_at) {
			continue;
		}
		rows.set(entry.job, {
			job: entry.job,
			inventory: mapping.inventory,
			state: entry.item.state,
			outcome: entry.item.outcome,
			created_at: entry.item.created_at
		});
	}
	return [...rows.values()].sort((left, right) => right.created_at - left.created_at);
}
