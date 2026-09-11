// The Analytics page's view logic: which marketplaces a scope covers, what a
// scope can honestly report, and the rows its chart and its table draw. Pure,
// so it tests without a component.
//
// One structural fact shapes the whole page. `/v1/analytics/summary` serves
// the newest captured figure per listing and nothing older
// (`crates/tam-api/src/analytics.rs:22-43`), and both device capture routes
// select bound listings for `InventoryId::Tpt` alone
// (`crates/tam-api/src/analytics.rs:242,280`). There is therefore no series
// over time to plot and no Tes figure to plot at all: the chart compares
// listings against each other rather than against a past, and a scope no
// marketplace reports for says so instead of drawing an empty axis.

import { METRIC_COLUMNS } from '$lib/analytics-view';
import type { ListingMetricsView, MappingHead, ProductHead } from '$lib/api';
import { agoLabel } from '$lib/elapsed';
import type { InventoryId, Marketplace } from '$lib/generated/vocab';
import type { IconName } from '$lib/icons';
import { INVENTORY_ORDER } from '$lib/listings-view';
import { SHORT_NAME } from '$lib/platforms';
import { IS_TES, standingOf, type PortfolioRow } from '$lib/tes-portfolio';

export type ScopeId = 'all' | 'tes' | 'tpt';

/** Which inventories each scope covers.
 *
 * A total map per scope rather than a predicate over the name, because the
 * inventory union is generated from the Rust enum: a marketplace added there
 * stops this file type-checking instead of falling silently outside every
 * scope. `all` covers Etsy although no scope of its own names it, because an
 * "All" that quietly omitted a marketplace would be a wrong total rather than
 * a missing tab.
 *
 * The TES row is `IS_TES` itself rather than a copy of it. The TES tab and the
 * portfolio panel draw the same counts from these two places, and while they
 * were separate maps nothing stopped them disagreeing and putting two totals
 * for one thing on one screen. */
const COVERAGE: Record<ScopeId, Record<InventoryId, boolean>> = {
	all: { Tpt: true, Tes: true, Etsy: true },
	tes: IS_TES,
	tpt: { Tpt: true, Tes: false, Etsy: false }
};

export function covers(scope: ScopeId, inventory: InventoryId): boolean {
	return COVERAGE[scope][inventory];
}

export interface Scope {
	id: ScopeId;
	label: string;
	/** What the tab narrows to, in the words its tooltip shows. */
	hint: string;
	/** The marketplace the tab draws as its mark, and null for the tab that
	 *  covers every marketplace and has no one mark to draw. */
	mark: Marketplace | null;
}

export const SCOPES: readonly Scope[] = [
	{ id: 'all', label: 'All', hint: 'Every marketplace your resources are set up for.', mark: null },
	{ id: 'tes', label: 'TES', hint: 'Your listings on TES.', mark: 'Tes' },
	{ id: 'tpt', label: 'TPT', hint: 'Your listings on Teachers Pay Teachers.', mark: 'Tpt' }
];

/** The scope a selector's value names, falling back to the widest.
 *
 * The selector hands back the string it was given, so this is where an
 * identifier becomes a scope. Written as a search over `SCOPES` rather than a
 * cast, so a scope removed from that list stops being reachable here rather
 * than being asserted into existence. */
export function asScope(id: string): ScopeId {
	for (const scope of SCOPES) {
		if (scope.id === id) {
			return scope.id;
		}
	}
	return 'all';
}

/** Which marketplaces serve figures of their own today.
 *
 * A statement about the backend rather than about a seller's data: the
 * capture routes read TPT alone, and the captured metric set is defined over
 * the TPT adapter (`crates/tam-analytics/src/lib.rs:31-35`). The page needs
 * this apart from the figures themselves, because a scope nobody reports for
 * and a scope whose first capture has not run are two different silences and
 * only one of them is worth waiting through. */
export const REPORTS_FIGURES: Record<InventoryId, boolean> = {
	Tpt: true,
	Tes: false,
	Etsy: false
};

/** Whether any marketplace in this scope reports figures at all. */
export function scopeReports(scope: ScopeId): boolean {
	return INVENTORY_ORDER.some(
		(inventory) => covers(scope, inventory) && REPORTS_FIGURES[inventory]
	);
}

/** The marketplaces in this scope that report nothing, named as the page
 *  names them, so the gap is stated in the seller's own vocabulary. */
export function silentIn(scope: ScopeId): string[] {
	const named = new Set<string>();
	for (const inventory of INVENTORY_ORDER) {
		if (covers(scope, inventory) && !REPORTS_FIGURES[inventory]) {
			named.add(SHORT_NAME[inventory]);
		}
	}
	return [...named];
}

/** How many mappings each scope covers, for the selector's counts. */
export function scopeCounts(mappings: readonly MappingHead[]): Record<ScopeId, number> {
	const counts: Record<ScopeId, number> = { all: 0, tes: 0, tpt: 0 };
	for (const mapping of mappings) {
		for (const scope of SCOPES) {
			if (covers(scope.id, mapping.inventory)) {
				counts[scope.id] += 1;
			}
		}
	}
	return counts;
}

export interface Figure {
	key: string;
	heading: string;
	/** The scope's total, or undefined where no listing in scope carries the
	 *  metric at all. Undefined is not zero: a marketplace that reports
	 *  nothing must never read as one that reported a nil. */
	total: number | undefined;
	/** How many listings contributed to the total, out of how many the scope
	 *  covers. A total drawn from 5 of 40 listings is a different fact from one
	 *  drawn from all 40, and without the pair they render identically. */
	from: number;
	of: number;
	/** The oldest instant among the listings that contributed, or null where
	 *  none did. The oldest rather than the newest: a sum is only as fresh as
	 *  its stalest part, and dating it by the newest would present a figure as
	 *  more current than it is. */
	asAt: number | null;
}

/** Each captured metric summed over the listings this scope covers.
 *
 * A total is the sum of the figures actually present. A listing carrying no
 * figure for a metric contributes nothing and is not counted in `from`, so a
 * total never quietly averages a marketplace's silence into a number. */
export function figures(
	listings: readonly ListingMetricsView[],
	scope: ScopeId
): Figure[] {
	const inScope = listings.filter((listing) => covers(scope, listing.inventory));
	return METRIC_COLUMNS.map((column) => {
		let total = 0;
		let from = 0;
		let asAt: number | null = null;
		for (const listing of inScope) {
			const value = listing.metrics[column.key];
			if (typeof value === 'number' && Number.isFinite(value)) {
				total += value;
				from += 1;
				if (asAt === null || listing.observed_at < asAt) {
					asAt = listing.observed_at;
				}
			}
		}
		return {
			key: column.key,
			heading: column.heading,
			total: from === 0 ? undefined : total,
			from,
			of: inScope.length,
			asAt
		};
	});
}

export interface Standing {
	listings: number;
	live: number;
	drafts: number;
	unsent: number;
	other: number;
}

/** Where this scope's listings stand, counted from the seller's own
 *  catalogue rather than reported by anyone. The four parts partition the
 *  total, which is what lets the chart draw them as shares of one whole. */
export function standings(mappings: readonly MappingHead[], scope: ScopeId): Standing {
	const standing: Standing = { listings: 0, live: 0, drafts: 0, unsent: 0, other: 0 };
	for (const mapping of mappings) {
		if (!covers(scope, mapping.inventory)) {
			continue;
		}
		standing.listings += 1;
		switch (standingOf(mapping)) {
			case 'live':
				standing.live += 1;
				break;
			case 'draft':
				standing.drafts += 1;
				break;
			case 'unsent':
				standing.unsent += 1;
				break;
			case 'other':
				standing.other += 1;
				break;
		}
	}
	return standing;
}

export interface ResourceRow {
	mapping: string;
	/** The catalogue's title for the product this mapping lists, or undefined
	 *  where the catalogue read does not name it. Undefined rather than a
	 *  stand-in: the table renders such a row by its identifier. */
	title: string | undefined;
	inventory: InventoryId;
	metrics: Record<string, number>;
	observedAt: number;
}

/** The metric the chart compares listings on.
 *
 * Views rather than sales or earnings: a listing accrues views whether or not
 * it sells, so this is the metric with the fewest listings sitting at nil,
 * and a chart of mostly-nil bars compares nothing. The table beneath carries
 * all three, so fixing this one hides no figure. A test holds it to a metric
 * the table has a column for. */
export const CHART_METRIC = 'resource_views';
export const CHART_HEADING = 'Item views';

function metricOf(row: ResourceRow, key: string): number {
	const value = row.metrics[key];
	return typeof value === 'number' && Number.isFinite(value) ? value : 0;
}

/** One row per listing this scope covers, best first.
 *
 * Ordered by the chart's metric, then by sales, then by identifier, so the
 * order is total and two renders of the same answer never disagree. */
export function resourceRows(
	listings: readonly ListingMetricsView[],
	titles: ReadonlyMap<string, string>,
	scope: ScopeId
): ResourceRow[] {
	const rows = listings
		.filter((listing) => covers(scope, listing.inventory))
		.map((listing) => ({
			mapping: listing.mapping,
			title: titles.get(listing.mapping),
			inventory: listing.inventory,
			metrics: listing.metrics,
			observedAt: listing.observed_at
		}));
	rows.sort(
		(left, right) =>
			metricOf(right, CHART_METRIC) - metricOf(left, CHART_METRIC) ||
			metricOf(right, 'sales_count') - metricOf(left, 'sales_count') ||
			left.mapping.localeCompare(right.mapping)
	);
	return rows;
}

/** A first approximation of how much of a bar's label will fit, used only for
 *  the frame before the chart has measured itself.
 *
 * A character count cannot decide this: thirty capital Ws are twice the width
 * of thirty lower-case ls, and a label clipped by count either overruns the
 * value or is cut short of the space it had. `MetricBars` measures the real
 * glyph widths and re-clips against the width it actually has; this is what it
 * draws for the one frame before that measurement exists. */
export const LABEL_MAX = 30;

export function clip(text: string, max: number = LABEL_MAX): string {
	return text.length <= max ? text : `${text.slice(0, max - 1).trimEnd()}…`;
}

export interface Bar {
	/** The text drawn beside the bar, at its full length. `MetricBars` decides
	 *  where to cut it, because only the drawing knows how much room it has. */
	label: string;
	/** What the bar means, for its tooltip. The same as the label where the
	 *  label is already the whole name, and an explanation where the label is a
	 *  short standing like "Live". */
	title: string;
	value: number;
	/** The bar's length as a fraction of the largest bar in the series. Zero
	 *  everywhere when the largest is itself nil, so a series of nils draws no
	 *  bar rather than drawing every bar full. */
	share: number;
}

function withShares(bars: readonly Omit<Bar, 'share'>[]): Bar[] {
	const largest = bars.reduce((most, bar) => Math.max(most, bar.value), 0);
	return bars.map((bar) => ({ ...bar, share: largest > 0 ? bar.value / largest : 0 }));
}

/** The chart's series where the scope has captured figures: the listings that
 *  carry the most of the charted metric, longest first. Listings carrying
 *  none of it are left out rather than drawn as a nil bar. */
export function topBars(rows: readonly ResourceRow[], limit: number): Bar[] {
	const named = rows
		.filter((row) => metricOf(row, CHART_METRIC) > 0)
		.slice(0, limit)
		.map((row) => {
			const name = row.title ?? row.mapping;
			return { label: name, title: name, value: metricOf(row, CHART_METRIC) };
		});
	return withShares(named);
}

/** The chart's series where the scope has no captured figures: where its
 *  listings stand, counted from the catalogue. */
export function standingBars(standing: Standing): Bar[] {
	const named = [
		{
			label: 'Live',
			title: 'The marketplace was last recorded showing this listing.',
			value: standing.live
		},
		{
			label: 'Drafts',
			title: 'Created on the marketplace and not published yet.',
			value: standing.drafts
		},
		{
			label: 'Not sent yet',
			title: 'Set up for a marketplace, with nothing created there yet.',
			value: standing.unsent
		},
		{
			label: 'In another state',
			title: 'Sent for review, in review, rejected, withdrawn, being created, or removed.',
			value: standing.other
		}
	];
	return withShares(named);
}



// --- the render model -----------------------------------------------------
//
// What the page draws, decided here rather than in the component, because the
// decisions the brief calls non-negotiable — an em dash is never a nil, a
// counted figure is never dressed as a reported one, a read that has not
// landed is never reported as an established fact — are exactly the ones a
// component-level test would have to reach, and this project's vitest
// environment is `node` with no DOM.

/** How a read the page depends on stands. */
export type ReadState = 'pending' | 'failed' | 'read';

export function readState(isSuccess: boolean, isError: boolean): ReadState {
	if (isError) {
		return 'failed';
	}
	return isSuccess ? 'read' : 'pending';
}

/** Two reads a figure needs, taken together, worst first.
 *
 * A figure narrowed by a label depends on the catalogue as well as on the
 * capture, and it is only as sound as the weaker of the two. */
export function combineReads(left: ReadState, right: ReadState): ReadState {
	if (left === 'failed' || right === 'failed') {
		return 'failed';
	}
	if (left === 'pending' || right === 'pending') {
		return 'pending';
	}
	return 'read';
}

/** What a tile draws where it has no figure. Never a nil: a marketplace that
 *  reported nothing and one that reported zero are different facts. */
const DASH = '—';

const TILE_ICON: Partial<Record<string, IconName>> = {
	sales_count: 'shopping-bag',
	earnings: 'credit-card',
	resource_views: 'activity'
};

export interface TileView {
	key: string;
	icon: IconName;
	label: string;
	/** The text drawn as the figure, already carrying the em dash where there
	 *  is none. The substitution lives here rather than in the component
	 *  because "an em dash, never a nil" is the rule this page is held to, and
	 *  a rule stated in markup is a rule nothing checks. */
	value: string;
	/** Whether `value` is a figure at all. False covers four different
	 *  silences, which `sub` then tells apart. */
	figure: boolean;
	tag?: string;
	sub: string;
	/** Counted from the seller's own catalogue rather than reported by a
	 *  marketplace, which the tile says in its colour as well as its words. */
	counted: boolean;
}

export interface TileInputs {
	figures: readonly Figure[];
	scope: ScopeId;
	/** How the capture read stands, already combined with the catalogue read
	 *  where a label narrows it. */
	summary: ReadState;
	/** How the two catalogue reads stand, taken together. */
	catalogue: ReadState;
	standing: Standing;
	now: number;
	format: (value: number) => string;
}

/** Every tile the page draws, in order.
 *
 * The order of the tests is the point. A marketplace that reports nothing is
 * answered first, because that is true whatever the capture read did; only
 * then does a failed or in-flight read speak, because either is a statement
 * about our own reading rather than about the seller's data; a captured
 * nothing comes last, and it is the only one of the four that asserts
 * anything about the tenant. */
export function tileViews(input: TileInputs): TileView[] {
	const reports = scopeReports(input.scope);
	const silent = silentIn(input.scope).join(' and ');
	const tag = input.scope === 'all' ? 'TPT only' : undefined;

	const reported = input.figures.map((figure): TileView => {
		const shell = {
			key: figure.key,
			icon: TILE_ICON[figure.key] ?? ('chart-line' as IconName),
			label: figure.heading,
			counted: false
		};
		const none = { ...shell, value: DASH, figure: false };
		if (!reports) {
			return { ...none, tag: 'not reported', sub: `${silent} reports no figures` };
		}
		if (input.summary === 'failed') {
			return { ...none, tag, sub: 'the analytics could not be read' };
		}
		if (input.summary === 'pending') {
			return { ...none, tag, sub: 'reading…' };
		}
		if (figure.total === undefined || figure.asAt === null) {
			return { ...none, tag, sub: 'nothing captured yet' };
		}
		// The contributor count is named only where it is short of the scope.
		// Saying "from 11 of 11" on every tile is noise; saying nothing when it
		// is 5 of 11 lets a partial total read as a complete one.
		const partial = figure.from < figure.of;
		const drawn = `as at ${agoLabel(figure.asAt, input.now)}`;
		return {
			...shell,
			value: input.format(figure.total),
			figure: true,
			sub: partial ? `${drawn}, from ${figure.from} of ${figure.of} listings` : drawn,
			tag
		};
	});

	const live: TileView = {
		key: 'live_listings',
		icon: 'store',
		label: 'Live listings',
		value: input.catalogue === 'read' ? input.format(input.standing.live) : DASH,
		figure: input.catalogue === 'read',
		tag: 'from your Resources',
		sub:
			input.catalogue === 'failed'
				? 'your Resources could not be read'
				: input.catalogue === 'pending'
					? 'counting…'
					: `of ${input.standing.listings} tracked`,
		counted: true
	};

	return [...reported, live];
}

export interface HeaderMeta {
	key: string;
	value: string;
	/** The instant the value describes, for the exact reading on hover; null
	 *  where the value is not an age at all. */
	at: number | null;
}

/** The header's freshness line.
 *
 * Dated by the OLDEST capture in scope, not the newest, so it agrees with
 * every tile and row beneath it. The newest instant is a real answer to a
 * different question — when an update last arrived — but printing it at the
 * top of a page whose figures are all older reads as a claim about those
 * figures, and it is the one place on this page that could say something
 * fresher than the truth. The label moves with the dating: "Last update" over
 * an oldest instant would be false. */
export function headerMeta(input: {
	scope: ScopeId;
	listings: readonly ListingMetricsView[];
	summary: ReadState;
	now: number;
}): HeaderMeta {
	const key = 'Figures as at';
	if (!scopeReports(input.scope)) {
		return { key, value: 'not reported', at: null };
	}
	if (input.summary === 'failed') {
		return { key, value: 'could not be read', at: null };
	}
	if (input.summary === 'pending') {
		return { key, value: 'reading…', at: null };
	}
	const oldest = oldestUpdate(input.listings, input.scope);
	if (oldest === null) {
		return { key, value: 'nothing yet', at: null };
	}
	return { key, value: agoLabel(oldest, input.now), at: oldest };
}

/** The oldest capture instant in scope, or null where the scope has none. */
export function oldestUpdate(
	listings: readonly ListingMetricsView[],
	scope: ScopeId
): number | null {
	let oldest: number | null = null;
	for (const listing of listings) {
		if (covers(scope, listing.inventory) && (oldest === null || listing.observed_at < oldest)) {
			oldest = listing.observed_at;
		}
	}
	return oldest;
}

export interface ChartView {
	title: string;
	description: string;
	bars: Bar[];
	/** Counted from the catalogue rather than reported, which sets the bars'
	 *  colour as well as the words around them. */
	counted: boolean;
	/** The drawing's accessible name. */
	label: string;
}

/** A panel's description, suppressed until the read behind it lands.
 *
 * A description tells the reader what the panel below is showing. Above "The
 * analytics could not be read" it describes something that is not there, which
 * is the same fault as a figure asserted from a failed read, in prose. */
function saidWhenRead(read: ReadState, description: string): string {
	return read === 'read' ? description : '';
}

export function chartView(input: {
	scope: ScopeId;
	rows: readonly ResourceRow[];
	standing: Standing;
	limit: number;
	/** How the capture read stands, for the reported branch. */
	summary: ReadState;
	/** How the catalogue reads stand, for the counted one. */
	catalogue: ReadState;
}): ChartView {
	if (scopeReports(input.scope)) {
		const bars = topBars(input.rows, input.limit);
		// The cap is named only where it actually bites. Saying "the 8 listings"
		// over a chart drawing two would be false, and a reader cannot tell a
		// cap from a total by looking at the bars.
		const capped = bars.length >= input.limit;
		const metric = CHART_HEADING.toLowerCase();
		return {
			title: 'Most viewed resources',
			description: saidWhenRead(
				input.summary,
				capped
					? `The ${input.limit} listings with the most ${metric}, longest bar first.`
					: `Your listings with any ${metric}, longest bar first.`
			),
			bars,
			counted: false,
			label: `${CHART_HEADING} per listing, largest first`
		};
	}
	const silent = silentIn(input.scope).join(' and ');
	return {
		title: 'Where your listings stand',
		description: saidWhenRead(
			input.catalogue,
			`Counted from your own Resources, because ${silent} publishes no figures.`
		),
		bars: standingBars(input.standing),
		counted: true,
		label: 'How many listings stand in each state'
	};
}

export interface PanelText {
	title: string;
	description: string;
}

/** The table panel's own wording.
 *
 * Conditional for the same reason the chart's is: on a scope no marketplace
 * reports for the panel holds a statement that there is nothing to rank, and a
 * description promising a ranked table over the top of it describes something
 * that is not there. */
export function tableView(scope: ScopeId, read: ReadState): PanelText {
	if (!scopeReports(scope)) {
		// Not conditioned on the read: this is true whatever the capture read
		// did, and it is the panel's whole content.
		return {
			title: 'Top resources',
			description: `Nothing here is ranked, because ${silentIn(scope).join(' and ')} reports no figures.`
		};
	}
	return {
		title: 'Top resources',
		description: saidWhenRead(
			read,
			`Your listings, best first by ${CHART_HEADING.toLowerCase()}, each with the age of its oldest figure.`
		)
	};
}

/** The standings panel's description, which the markup holds rather than the
 *  model because the two variants differ only in their words. */
export function standingsDescription(read: ReadState, description: string): string {
	return saidWhenRead(read, description);
}

export interface PortfolioGroup {
	row: PortfolioRow;
	/** Rows that break this one down rather than standing beside it. */
	under: PortfolioRow[];
}

/** The portfolio rows nested so a sub-count sits under the figure it comes
 *  out of, never beside it as though the two were peers. */
export function groupPortfolioRows(rows: readonly PortfolioRow[]): PortfolioGroup[] {
	const groups: PortfolioGroup[] = [];
	for (const row of rows) {
		const parent = groups.at(-1);
		if (row.breakdown === true && parent !== undefined) {
			parent.under.push(row);
		} else {
			groups.push({ row, under: [] });
		}
	}
	return groups;
}

/** The mappings whose product survived a narrowed catalogue read.
 *
 * A label narrows the catalogue at the server, so the products that come back
 * are the whole answer to "which resources carry this label"; everything else
 * on the page follows from which mappings point at them. */
export function mappingsForProducts(
	mappings: readonly MappingHead[],
	products: readonly ProductHead[]
): MappingHead[] {
	const kept = new Set(products.map((product) => product.id));
	return mappings.filter((mapping) => kept.has(mapping.product));
}

/** The captured listings belonging to those mappings. */
export function listingsForMappings(
	listings: readonly ListingMetricsView[],
	mappings: readonly MappingHead[]
): ListingMetricsView[] {
	const kept = new Set(mappings.map((mapping) => mapping.id));
	return listings.filter((listing) => kept.has(listing.mapping));
}
