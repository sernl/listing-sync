// Every closed wire enum, every value, against the board's pure functions.
//
// The board reads five queries and builds every row from one derived
// expression, so a single unhandled value on a single mapping does not degrade
// one chip: it throws, the render boundary catches it, and the seller reads a
// sentence where their catalogue was. The dev organisation had no connections,
// no jobs and no halts, which is exactly the shape that leaves the run-derived
// half of this module never executed against real data.
//
// The oracle here is deliberately weak — nothing throws, and a chip is
// well-formed — because the failure being guarded is a throw. It is not
// coverage of whether the chips say true things; the cases in
// `inventory.test.ts` and the rest of this file's siblings do that, and this
// sweep does not extend them.
//
// The enumeration reads the runtime arrays `tam-typegen` emits beside each
// union, so an enum widened in Rust widens the sweep on the next `web-typegen`
// rather than leaving it quietly narrower than the vocabulary it claims to
// cover. Enumerated rather than sampled because the space is closed and finite;
// sampling it would want `fast-check`, which is a dependency decision and not a
// test-file one.

import { describe, expect, it } from 'vitest';
import {
	ATTENTION_STATES,
	STATE_LABEL,
	STATE_TONE,
	chipFor,
	rowFor,
	type ChipInput,
	type InventoryRow,
	type MarketplaceChip,
	type WorkItem
} from '$lib/inventory';
import { tileFor } from './marketplace-tile';
import {
	NO_RESOURCE_FILTERS,
	listedOn,
	matchesResource,
	metaLine,
	needsYou,
	sortRows,
	SORTS,
	standingOfRow,
	tabCounts,
	type ResourceFilters
} from './list';
import { INVENTORY_ORDER } from '$lib/listings-view';
import type {
	ConnectionView,
	InventoryStatus,
	ItemView,
	MappingHead,
	ProductHead
} from '$lib/api';
import {
	BLOCKED_GATES,
	CONNECTION_STATUSES,
	INVENTORY_IDS,
	ITEM_OUTCOMES,
	ITEM_STATES,
	type BlockedGate,
	type ConnectionStatus,
	type InventoryId,
	type ItemOutcome,
	type ItemState
} from '$lib/generated/vocab';

const TONES = new Set(Object.values(STATE_TONE));
const STATES = new Set(Object.keys(STATE_LABEL));

/** The four standings `standingOf` partitions a mapping into, each written as
 *  the pair of stored spellings that produces it. The spellings are the
 *  server's own strings rather than a generated union, so the last pair is a
 *  shape this client does not recognise and must still carry. */
const STANDINGS: readonly { name: string; binding: string; lifecycle: string }[] = [
	{ name: 'live', binding: 'bound', lifecycle: 'live' },
	{ name: 'draft', binding: 'bound', lifecycle: 'draft' },
	{ name: 'unsent', binding: 'unbound', lifecycle: 'absent' },
	{ name: 'other', binding: 'binding', lifecycle: 'whatever-this-is' }
];

/** Absent as well as present, everywhere the wire makes a field optional. */
const OUTCOMES: readonly (ItemOutcome | undefined)[] = [undefined, ...ITEM_OUTCOMES];
const GATES: readonly (BlockedGate | undefined)[] = [undefined, ...BLOCKED_GATES];
const STATUSES: readonly (ConnectionStatus | undefined)[] = [undefined, ...CONNECTION_STATUSES];

function mappingOf(inventory: InventoryId, standing: (typeof STANDINGS)[number]): MappingHead {
	return {
		id: `m-${inventory}`,
		product: 'p1',
		inventory,
		binding_state: standing.binding,
		lifecycle_state: standing.lifecycle,
		updated_at: 10,
		listing_url: standing.name === 'live' ? 'https://example.test/listing' : null
	};
}

function workOf(state: ItemState, outcome: ItemOutcome | undefined, gate: BlockedGate | undefined) {
	const item: ItemView = {
		item: 'i1',
		mapping: 'm1',
		state,
		attempt_count: 1,
		created_at: 5,
		...(outcome === undefined ? {} : { outcome }),
		...(gate === undefined ? {} : { blocked_on: gate })
	};
	return { job: 'j1', item } satisfies WorkItem;
}

function connectionOf(
	inventory: InventoryId,
	status: ConnectionStatus | undefined
): ConnectionView | undefined {
	if (status === undefined) {
		return undefined;
	}
	return {
		id: `c-${inventory}`,
		marketplace: inventory === 'Tpt' ? 'Tpt' : inventory === 'Etsy' ? 'Etsy' : 'Tes',
		state: 'linked',
		status,
		created_at: 1,
		updated_at: 2
	};
}

function statusOf(inventory: InventoryId, halted: boolean): InventoryStatus | undefined {
	if (!halted) {
		return undefined;
	}
	return {
		inventory,
		marketplace: inventory === 'Tpt' ? 'Tpt' : inventory === 'Etsy' ? 'Etsy' : 'Tes',
		halted: true,
		raised_at: 3
	};
}

/** What is wrong with this chip, or nothing.
 *
 * Collected into a list and asserted once rather than through an `expect` per
 * field: forty thousand cases times five matchers is slower than the whole
 * suite, and a list names every offending case instead of stopping at the
 * first. */
function faultsIn(chip: MarketplaceChip, where: string): string[] {
	const faults: string[] = [];
	if (!STATES.has(chip.state)) {
		faults.push(`${where}: state ${chip.state} is outside the console's vocabulary`);
	}
	if (typeof chip.label !== 'string' || chip.label.length === 0) {
		faults.push(`${where}: no label`);
	}
	if (!TONES.has(chip.tone)) {
		faults.push(`${where}: tone ${String(chip.tone)} is outside the four`);
	}
	if (typeof chip.detail !== 'string' || chip.detail.trim().length === 0) {
		faults.push(`${where}: no detail sentence`);
	}
	if (chip.action !== null && chip.action.href.length === 0) {
		faults.push(`${where}: an action with nowhere to go`);
	}
	// A seller sent to the Marketplaces page arrives at a grid of twenty-three
	// cards, so every verdict that sends them there names the marketplace it
	// sent them for. Held over the whole space rather than at the four sites
	// that reach it today, because the gate router has a fifth arm that no gate
	// currently routes to and a gate added in Rust could.
	if (chip.action !== null && chip.action.href === '/marketplaces') {
		faults.push(`${where}: sent to the Marketplaces page with no marketplace named`);
	}
	return faults;
}

/** What is wrong with the tile that chip draws, or nothing.
 *
 * The same weak oracle and for the same reason: the resource page builds every
 * tile from one derived expression, so a single state `tileFor` has no arm for
 * empties the panel rather than one tile. It is not coverage of whether a tile
 * says a true thing; `marketplace-tile.test.ts` does that. */
function tileFaultsIn(chip: MarketplaceChip, mapping: MappingHead | undefined, where: string): string[] {
	const tile = tileFor({ chip, mapping, product: 'p1', now: 20, readiness: undefined });
	const faults: string[] = [];
	if (tile.label.length === 0) {
		faults.push(`${where}: a tile with no status word`);
	}
	if (tile.detail.trim().length === 0) {
		faults.push(`${where}: a tile with no clause under the word`);
	}
	if (tile.markSrc.length === 0) {
		faults.push(`${where}: a tile with no mark`);
	}
	if (tile.name.trim().length === 0) {
		faults.push(`${where}: a tile with no accessible name`);
	}
	if (tile.href !== null && tile.href === '/resources/p1') {
		faults.push(`${where}: a tile linking back to the page it is drawn on`);
	}
	return faults;
}

describe('every chip the closed enums can produce', () => {
	// One mapping standing rather than four, because the run outranks the
	// standing whenever it says anything: the standings get their own sweep
	// below, where the work is absent and they decide the chip.
	const live = STANDINGS[0];

	const chips: { chip: MarketplaceChip; where: string }[] = [];
	for (const state of ITEM_STATES) {
		for (const outcome of OUTCOMES) {
			for (const gate of GATES) {
				for (const inventory of INVENTORY_IDS) {
					for (const status of STATUSES) {
						for (const halted of [false, true]) {
							const input: ChipInput = {
								product: 'p1',
								inventory,
								mapping: mappingOf(inventory, live),
								work: workOf(state, outcome, gate),
								connection: connectionOf(inventory, status),
								status: statusOf(inventory, halted)
							};
							chips.push({
								chip: chipFor(input),
								where: `${state}/${String(outcome)}/${String(gate)}/${inventory}/${String(status)}/halted=${halted}`
							});
						}
					}
				}
			}
		}
	}

	it('enumerates the whole product, so an empty sweep cannot pass silently', () => {
		expect(chips.length).toBe(
			ITEM_STATES.length *
				OUTCOMES.length *
				GATES.length *
				INVENTORY_IDS.length *
				STATUSES.length *
				2
		);
		expect(chips.length).toBeGreaterThan(30000);
	});

	it('answers with a well-formed chip for every one of them', () => {
		expect(chips.flatMap(({ chip, where }) => faultsIn(chip, where))).toEqual([]);
	});

	it('names the marketplace it was asked about, every time', () => {
		const wrong = chips.filter(({ chip, where }) => !where.includes(`/${chip.inventory}/`));
		expect(wrong.map(({ where }) => where)).toEqual([]);
	});

	it('draws a well-formed tile from every one of them', () => {
		expect(
			chips.flatMap(({ chip, where }) =>
				tileFaultsIn(chip, mappingOf(chip.inventory, live), where)
			)
		).toEqual([]);
	});
});

describe('every chip the stored standings can produce', () => {
	const chips: { chip: MarketplaceChip; mapping: MappingHead | undefined; where: string }[] = [];
	for (const standing of [...STANDINGS, undefined]) {
		for (const inventory of INVENTORY_IDS) {
			for (const status of STATUSES) {
				for (const halted of [false, true]) {
					const mapping =
						standing === undefined ? undefined : mappingOf(inventory, standing);
					chips.push({
						chip: chipFor({
							product: 'p1',
							inventory,
							mapping,
							work: undefined,
							connection: connectionOf(inventory, status),
							status: statusOf(inventory, halted)
						}),
						mapping,
						where: `${standing?.name ?? 'unmapped'}/${inventory}/${String(status)}/halted=${halted}`
					});
				}
			}
		}
	}

	it('enumerates every standing including the absent mapping', () => {
		expect(chips.length).toBe(5 * INVENTORY_IDS.length * STATUSES.length * 2);
		expect(chips.length).toBeGreaterThan(100);
	});

	it('answers with a well-formed chip for every one of them', () => {
		expect(chips.flatMap(({ chip, where }) => faultsIn(chip, where))).toEqual([]);
	});

	it('draws a well-formed tile from every one of them, mapped or not', () => {
		expect(chips.flatMap(({ chip, mapping, where }) => tileFaultsIn(chip, mapping, where))).toEqual(
			[]
		);
	});
});

// The regression this whole file was written for. `ofWork` switched over
// `ItemState` with no final arm, so a state outside the union returned
// `undefined` through a `Verdict | null` signature, the `!== null` test let it
// past, and reading `.state` off it threw before the first paint. The console
// is a static bundle served from the control plane, so a Rust deploy that
// widens the enum reaches a seller whose page was built against the narrower
// one; that is the window this case stands in for.
describe('a state this bundle has no word for', () => {
	const beyond = 'reticulating_splines' as ItemState;

	it('is a chip rather than a thrown page', () => {
		const chip = chipFor({
			product: 'p1',
			inventory: 'Tpt',
			mapping: mappingOf('Tpt', STANDINGS[0]),
			work: workOf(beyond, undefined, undefined),
			connection: connectionOf('Tpt', 'connected'),
			status: undefined
		});
		expect(faultsIn(chip, 'beyond')).toEqual([]);
		expect(chip.detail).toContain('does not have a word for yet');
		expect(chip.action).toEqual({ label: 'Open the run', href: '/sync/j1' });
	});

	it('takes one chip down rather than the row it sits in', () => {
		const row = rowFor({
			product: productOf(),
			mappings: INVENTORY_IDS.map((inventory) => mappingOf(inventory, STANDINGS[0])),
			work: new Map(
				INVENTORY_IDS.map((inventory) => [
					`m-${inventory}`,
					workOf(beyond, undefined, undefined)
				])
			),
			connections: [],
			statuses: []
		});
		expect(row.chips.length).toBe(INVENTORY_IDS.length);
		expect(row.chips.flatMap((chip) => faultsIn(chip, chip.inventory))).toEqual([]);
	});
});

function productOf(partial: Partial<ProductHead> = {}): ProductHead {
	return {
		id: 'p1',
		title: 'Fractions pack',
		price: 'Free',
		created_at: 1,
		updated_at: 2,
		...partial
	};
}

/** Rows built across the same closed space, one mapping each, so a row's
 *  consistency is checked against every chip the sweep above can produce. */
function sweptRows(): { row: InventoryRow; where: string }[] {
	const rows: { row: InventoryRow; where: string }[] = [];
	for (const inventory of INVENTORY_IDS) {
		for (const standing of STANDINGS) {
			for (const state of ITEM_STATES) {
				for (const status of STATUSES) {
					for (const halted of [false, true]) {
						const mapping = mappingOf(inventory, standing);
						const connection = connectionOf(inventory, status);
						rows.push({
							row: rowFor({
								product: productOf({ id: `p-${inventory}-${state}` }),
								mappings: [mapping],
								work: new Map([[mapping.id, workOf(state, undefined, undefined)]]),
								connections: connection === undefined ? [] : [connection],
								statuses: [statusOf(inventory, halted)].filter(
									(entry): entry is InventoryStatus => entry !== undefined
								)
							}),
							where: `${inventory}/${standing.name}/${state}/${String(status)}/halted=${halted}`
						});
					}
				}
			}
		}
	}
	return rows;
}

describe('every row those chips can build', () => {
	const rows = sweptRows();

	it('enumerates the whole product, so an empty sweep cannot pass silently', () => {
		expect(rows.length).toBe(
			INVENTORY_IDS.length * STANDINGS.length * ITEM_STATES.length * STATUSES.length * 2
		);
		expect(rows.length).toBeGreaterThan(500);
	});

	it('draws one chip per marketplace on the strip, never two', () => {
		const wrong = rows.filter(
			({ row }) => new Set(row.chips.map((chip) => chip.inventory)).size !== row.chips.length
		);
		expect(wrong.map(({ where }) => where)).toEqual([]);
	});

	it('shows only marketplaces the strip order names', () => {
		const wrong = rows.filter(({ row }) =>
			row.chips.some((chip) => !INVENTORY_ORDER.includes(chip.inventory))
		);
		expect(wrong.map(({ where }) => where)).toEqual([]);
	});

	it('holds an attention list that is exactly the chips needing attention', () => {
		const wrong = rows.filter(({ row }) => {
			const expected = row.chips.filter((chip) => ATTENTION_STATES.includes(chip.state));
			return (
				row.attention.length !== expected.length ||
				row.attention.some((chip) => !expected.includes(chip))
			);
		});
		expect(wrong.map(({ where }) => where)).toEqual([]);
	});

	it('keys its mapped set by the marketplaces the mappings name', () => {
		const wrong = rows.filter(({ row }) => {
			const keys = [...row.mapped.keys()];
			return keys.length !== 1 || row.mapped.get(keys[0])?.inventory !== keys[0];
		});
		expect(wrong.map(({ where }) => where)).toEqual([]);
	});
});

describe('the board functions over every row the sweep builds', () => {
	const rows = sweptRows().map(({ row }) => row);

	it('partitions the tab counts, so the three standings sum to all', () => {
		const counts = tabCounts(rows);
		expect(counts.all).toBe(rows.length);
		expect(counts.not_listed + counts.draft + counts.listed).toBe(counts.all);
		expect(counts.attention).toBeLessThanOrEqual(counts.all);
	});

	it('answers a standing and an attention verdict for each of them', () => {
		const faults = rows.flatMap((row) => {
			const standing = standingOfRow(row);
			const listed = row.chips.some((chip) => chip.state === 'listed');
			if (listed && standing !== 'listed') {
				return [`${row.product.id}: a listed chip and a standing of ${standing}`];
			}
			if (needsYou(row) !== row.attention.length > 0) {
				return [`${row.product.id}: needsYou disagrees with the attention list`];
			}
			return [];
		});
		expect(faults).toEqual([]);
	});

	it('names a marketplace for every listed chip and none for the rest', () => {
		const faults = rows.flatMap((row) => {
			const listed = row.chips.filter((chip) => chip.state === 'listed');
			const names = listedOn(row);
			if (names.length !== listed.length) {
				return [`${row.product.id}: ${names.length} names for ${listed.length} listed chips`];
			}
			return names.some((name) => name.length === 0)
				? [`${row.product.id}: an empty marketplace name`]
				: [];
		});
		expect(faults).toEqual([]);
	});

	// Every tab against every standing, because `inTab` and the standing filter
	// compose and a row that no tab admits would disappear from the board with
	// nothing saying where it went.
	it('admits every row under some tab', () => {
		const filters: readonly ResourceFilters[] = [
			NO_RESOURCE_FILTERS,
			{ ...NO_RESOURCE_FILTERS, tab: 'not_listed' },
			{ ...NO_RESOURCE_FILTERS, tab: 'draft' },
			{ ...NO_RESOURCE_FILTERS, tab: 'listed' },
			{ ...NO_RESOURCE_FILTERS, tab: 'attention' },
			{ ...NO_RESOURCE_FILTERS, standing: 'attention' },
			{ ...NO_RESOURCE_FILTERS, marketplaces: [...INVENTORY_IDS] },
			{ ...NO_RESOURCE_FILTERS, query: 'fractions' }
		];
		const homeless = rows.filter(
			(row) => !filters.some((filter) => matchesResource(row, filter))
		);
		expect(homeless.map((row) => row.product.id)).toEqual([]);
	});

	it('sorts without losing or inventing a row, under every order', () => {
		const faults = SORTS.flatMap(({ id }) => {
			const sorted = sortRows(rows, id);
			if (sorted.length !== rows.length) {
				return [`${id}: ${sorted.length} rows from ${rows.length}`];
			}
			return new Set(sorted).size === rows.length ? [] : [`${id}: a row appears twice`];
		});
		expect(faults).toEqual([]);
	});
});

describe('the meta line over every price shape the wire can carry', () => {
	// The three `PriceIntent` serialises as, then five it never does: the
	// products endpoint types this field `unknown` on purpose, so the shapes
	// below are what a drifted server, a partial response or a null column
	// would put in front of this function.
	const PRICES: readonly unknown[] = [
		'Free',
		{ Paid: { minor_units: 499, currency: 'Gbp' } },
		{ Paid: { minor_units: 1200, currency: 'Usd' } },
		null,
		undefined,
		42,
		'Whatever',
		{ Paid: { minor_units: 'four ninety nine', currency: 'Gbp' } },
		{ Paid: { minor_units: 499, currency: 'Xyz' } }
	];

	const COVERS: readonly (string | null | undefined)[] = [undefined, null, 'https://cover.test/a'];

	it('reads a sentence for every price and cover, present or absent', () => {
		const row = rowFor({
			product: productOf(),
			mappings: [mappingOf('Tpt', STANDINGS[0])],
			work: new Map(),
			connections: [],
			statuses: []
		});
		const lines: string[] = [];
		for (const price of PRICES) {
			for (const cover of COVERS) {
				const head = productOf({ price, ...(cover === undefined ? {} : { cover }) });
				lines.push(metaLine(head, row, 1_000_000));
			}
		}
		expect(lines.length).toBe(PRICES.length * COVERS.length);
		expect(lines.filter((line) => line.trim().length === 0)).toEqual([]);
		expect(lines.every((line) => line.startsWith('Updated '))).toBe(true);
	});
});
