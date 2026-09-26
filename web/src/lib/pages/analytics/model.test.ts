import { describe, expect, it } from 'vitest';
import { METRIC_COLUMNS } from '$lib/analytics-view';
import type { ListingMetricsView, MappingHead } from '$lib/api';
import type { InventoryId } from '$lib/generated/vocab';
import { IS_TES, PORTFOLIO_ROWS, tesPortfolio } from '$lib/tes-portfolio';
import {
	CHART_METRIC,
	CHART_HEADING,
	LABEL_MAX,
	asScope,
	chartView,
	clip,
	tableView,
	combineReads,
	groupPortfolioRows,
	headerMeta,
	listingsForMappings,
	mappingsForProducts,
	readState,
	tileViews,
	type Figure,
	type ReadState,
	type Standing,
	covers,
	figures,
	oldestUpdate,
	resourceRows,
	scopeCounts,
	scopeReports,
	silentIn,
	standingBars,
	standings,
	topBars
} from './model';

const HOUR = 3_600_000;

function listing(
	mapping: string,
	inventory: InventoryId,
	metrics: Record<string, number>,
	observedAt = 0
): ListingMetricsView {
	return { mapping, inventory, observed_at: observedAt, metrics };
}

function mapping(
	id: string,
	inventory: InventoryId,
	binding = 'bound',
	lifecycle = 'live'
): MappingHead {
	return {
		id,
		product: `p-${id}`,
		inventory,
		binding_state: binding,
		lifecycle_state: lifecycle,
		updated_at: 0,
		listing_url: null
	};
}

describe('the marketplace scopes', () => {
	it('puts Tes under TES and TPT under TPT', () => {
		expect(covers('tes', 'Tes')).toBe(true);
		expect(covers('tes', 'Tpt')).toBe(false);
		expect(covers('tpt', 'Tpt')).toBe(true);
		expect(covers('tpt', 'Tes')).toBe(false);
	});

	it('covers every marketplace under All, including one with no tab of its own', () => {
		const inventories: InventoryId[] = ['Tpt', 'Tes', 'Etsy'];
		for (const inventory of inventories) {
			expect(covers('all', inventory)).toBe(true);
		}
	});

	it('reads a selector value back as a scope, and an unknown one as All', () => {
		expect(asScope('tes')).toBe('tes');
		expect(asScope('tpt')).toBe('tpt');
		expect(asScope('etsy')).toBe('all');
	});

	it('counts a mapping under All and under its own marketplace', () => {
		const counts = scopeCounts([
			mapping('a', 'Tpt'),
			mapping('b', 'Tes'),
			mapping('c', 'Etsy')
		]);
		expect(counts).toEqual({ all: 3, tes: 1, tpt: 1 });
	});
});

describe('which scopes report figures', () => {
	it('says TPT and All report and TES does not', () => {
		expect(scopeReports('tpt')).toBe(true);
		expect(scopeReports('all')).toBe(true);
		expect(scopeReports('tes')).toBe(false);
	});

	it('names the silent marketplaces once each, whatever the site count', () => {
		expect(silentIn('tes')).toEqual(['TES']);
		expect(silentIn('all')).toEqual(['TES', 'Etsy']);
		expect(silentIn('tpt')).toEqual([]);
	});
});

describe('the scope figures', () => {
	const captured = [
		listing('m1', 'Tpt', { sales_count: 2, earnings: 10.5, resource_views: 100 }),
		listing('m2', 'Tpt', { sales_count: 3, resource_views: 40 })
	];

	it('sums each metric over the listings the scope covers', () => {
		const summed = figures(captured, 'tpt');
		expect(summed.map((figure) => [figure.key, figure.total])).toEqual([
			['sales_count', 5],
			['earnings', 10.5],
			['resource_views', 140]
		]);
	});

	it('counts only the listings that carried the metric', () => {
		const summed = figures(captured, 'tpt');
		expect(summed.find((figure) => figure.key === 'earnings')?.from).toBe(1);
		expect(summed.find((figure) => figure.key === 'sales_count')?.from).toBe(2);
	});

	it('answers undefined rather than nil where nothing in scope carries a figure', () => {
		for (const figure of figures(captured, 'tes')) {
			expect(figure.total).toBeUndefined();
			expect(figure.from).toBe(0);
			expect(figure.asAt).toBeNull();
		}
	});

	it('dates a total by the oldest figure in it, never the newest', () => {
		const staggered = [
			listing('m1', 'Tpt', { sales_count: 1 }, 9 * HOUR),
			listing('m2', 'Tpt', { sales_count: 1 }, 2 * HOUR)
		];
		expect(figures(staggered, 'tpt')[0].asAt).toBe(2 * HOUR);
	});

	it('dates each metric by its own contributors, not by the whole read', () => {
		const staggered = [
			listing('m1', 'Tpt', { sales_count: 1 }, HOUR),
			listing('m2', 'Tpt', { earnings: 5 }, 8 * HOUR)
		];
		const summed = figures(staggered, 'tpt');
		expect(summed.find((figure) => figure.key === 'sales_count')?.asAt).toBe(HOUR);
		expect(summed.find((figure) => figure.key === 'earnings')?.asAt).toBe(8 * HOUR);
	});

	it('ignores a figure that is not a finite number', () => {
		const broken = [listing('m1', 'Tpt', { sales_count: Number.NaN })];
		expect(figures(broken, 'tpt')[0].total).toBeUndefined();
	});
});

describe('where the listings stand', () => {
	const mappings = [
		mapping('a', 'Tes'),
		mapping('b', 'Tes', 'bound', 'draft'),
		mapping('c', 'Tes', 'unbound', 'draft'),
		mapping('d', 'Tes', 'bound', 'in_review'),
		mapping('e', 'Tpt')
	];

	it('partitions the scope: the four parts sum to the total', () => {
		const standing = standings(mappings, 'tes');
		expect(standing).toEqual({ listings: 4, live: 1, drafts: 1, unsent: 1, other: 1 });
		expect(standing.live + standing.drafts + standing.unsent + standing.other).toBe(
			standing.listings
		);
	});

	it('leaves out the marketplaces the scope does not cover', () => {
		expect(standings(mappings, 'tpt').listings).toBe(1);
	});
});

describe('the resource rows', () => {
	const titles = new Map([
		['m1', 'Fractions pack'],
		['m2', 'Place value mats']
	]);
	const captured = [
		listing('m2', 'Tpt', { resource_views: 40, sales_count: 1 }, 3 * HOUR),
		listing('m1', 'Tpt', { resource_views: 100, sales_count: 2 }, HOUR),
		listing('m3', 'Tes', { resource_views: 999 }, HOUR)
	];

	it('orders by the charted metric, best first', () => {
		expect(resourceRows(captured, titles, 'tpt').map((row) => row.mapping)).toEqual([
			'm1',
			'm2'
		]);
	});

	it('carries the inventory and leaves an unknown title undefined', () => {
		const rows = resourceRows(captured, titles, 'all');
		const unknown = rows.find((row) => row.mapping === 'm3');
		expect(unknown?.title).toBeUndefined();
		expect(unknown?.inventory).toBe('Tes');
	});

	it('breaks a tie on the charted metric by sales, then by identifier', () => {
		const tied = [
			listing('b', 'Tpt', { resource_views: 5, sales_count: 1 }),
			listing('a', 'Tpt', { resource_views: 5, sales_count: 1 }),
			listing('c', 'Tpt', { resource_views: 5, sales_count: 9 })
		];
		expect(resourceRows(tied, titles, 'tpt').map((row) => row.mapping)).toEqual([
			'c',
			'a',
			'b'
		]);
	});
});

describe('the chart series', () => {
	it('charts a metric the table has a column for', () => {
		const column = METRIC_COLUMNS.find((candidate) => candidate.key === CHART_METRIC);
		expect(column).toBeDefined();
		expect(column?.heading).toBe(CHART_HEADING);
	});

	it('scales every bar against the longest one', () => {
		const rows = resourceRows(
			[
				listing('m1', 'Tpt', { resource_views: 100 }),
				listing('m2', 'Tpt', { resource_views: 25 })
			],
			new Map(),
			'tpt'
		);
		expect(topBars(rows, 5).map((bar) => bar.share)).toEqual([1, 0.25]);
	});

	it('leaves out a listing carrying none of the charted metric', () => {
		const rows = resourceRows(
			[
				listing('m1', 'Tpt', { resource_views: 3 }),
				listing('m2', 'Tpt', { sales_count: 8 })
			],
			new Map(),
			'tpt'
		);
		expect(topBars(rows, 5).map((bar) => bar.label)).toEqual(['m1']);
	});

	it('keeps to the limit it is given', () => {
		const rows = resourceRows(
			[1, 2, 3, 4, 5].map((n) => listing(`m${n}`, 'Tpt', { resource_views: n })),
			new Map(),
			'tpt'
		);
		expect(topBars(rows, 3)).toHaveLength(3);
	});

	it('draws no bar at all when every figure in the series is nil', () => {
		expect(
			standingBars({ listings: 0, live: 0, drafts: 0, unsent: 0, other: 0 }).map(
				(bar) => bar.share
			)
		).toEqual([0, 0, 0, 0]);
	});

	// The drawing truncates `label` and hovers `title`. A series that put its
	// explanation in `label` would have the chart draw a sentence where a word
	// belongs, which is exactly what happened once.
	it('draws the short standing and explains it only on hover', () => {
		const bars = standingBars({ listings: 4, live: 1, drafts: 1, unsent: 1, other: 1 });
		expect(bars.map((bar) => bar.label)).toEqual([
			'Live',
			'Drafts',
			'Not sent yet',
			'In another state'
		]);
		for (const bar of bars) {
			expect(bar.title.length).toBeGreaterThan(bar.label.length);
		}
	});

	it('draws a resource under its own name, with nothing else to explain', () => {
		const rows = resourceRows(
			[listing('m1', 'Tpt', { resource_views: 3 })],
			new Map([['m1', 'Fractions pack']]),
			'tpt'
		);
		expect(topBars(rows, 5)[0]).toMatchObject({
			label: 'Fractions pack',
			title: 'Fractions pack'
		});
	});

	it('names the four standings in the order the panel counts them', () => {
		const bars = standingBars({ listings: 6, live: 3, drafts: 2, unsent: 1, other: 0 });
		expect(bars.map((bar) => [bar.label, bar.value])).toEqual([
			['Live', 3],
			['Drafts', 2],
			['Not sent yet', 1],
			['In another state', 0]
		]);
	});
});

describe('clipping a bar label', () => {
	it('leaves a label that already fits', () => {
		expect(clip('Fractions pack')).toBe('Fractions pack');
	});

	it('clips a longer one to the width the phone can draw', () => {
		const clipped = clip('a'.repeat(LABEL_MAX + 10));
		expect(clipped).toHaveLength(LABEL_MAX);
		expect(clipped.endsWith('…')).toBe(true);
	});
});

describe('the oldest update', () => {
	it('is the oldest instant in the scope, which is what the header dates by', () => {
		const captured = [
			listing('m1', 'Tpt', {}, 5 * HOUR),
			listing('m2', 'Tpt', {}, 9 * HOUR),
			listing('m3', 'Tes', {}, HOUR)
		];
		expect(oldestUpdate(captured, 'tpt')).toBe(5 * HOUR);
		expect(oldestUpdate(captured, 'all')).toBe(HOUR);
	});

	it('is null where the scope has heard nothing', () => {
		expect(oldestUpdate([listing('m1', 'Tpt', {}, HOUR)], 'tes')).toBeNull();
	});
});


// --- the render model -----------------------------------------------------

const INVENTORIES: InventoryId[] = ['Tpt', 'Tes', 'Etsy'];
const NOTHING: Standing = { listings: 0, live: 0, drafts: 0, unsent: 0, other: 0 };

function figure(
	key: string,
	total: number | undefined,
	asAt: number | null,
	of = 1
): Figure {
	return { key, heading: key, total, from: total === undefined ? 0 : 1, of, asAt };
}

/** The three the page always draws, as `figures` returns them. */
function threeFigures(total: number | undefined, asAt: number | null, of = 1): Figure[] {
	return [
		figure('sales_count', total, asAt, of),
		figure('earnings', total, asAt, of),
		figure('resource_views', total, asAt, of)
	];
}

function tiles(over: {
	scope?: 'all' | 'tes' | 'tpt';
	summary?: ReadState;
	catalogue?: ReadState;
	figures?: Figure[];
	standing?: Standing;
}) {
	return tileViews({
		figures: over.figures ?? threeFigures(10, 0),
		scope: over.scope ?? 'tpt',
		summary: over.summary ?? 'read',
		catalogue: over.catalogue ?? 'read',
		standing: over.standing ?? NOTHING,
		now: 2 * HOUR,
		format: (value) => String(value)
	});
}

describe('reading a query state', () => {
	it('is failed on error, read on success, pending otherwise', () => {
		expect(readState(false, true)).toBe('failed');
		expect(readState(true, false)).toBe('read');
		expect(readState(false, false)).toBe('pending');
	});

	it('prefers the error even where a stale success is also reported', () => {
		expect(readState(true, true)).toBe('failed');
	});

	it('combines two reads to the weaker of them', () => {
		expect(combineReads('read', 'read')).toBe('read');
		expect(combineReads('read', 'pending')).toBe('pending');
		expect(combineReads('pending', 'failed')).toBe('failed');
		expect(combineReads('failed', 'read')).toBe('failed');
	});
});

describe('the figure tiles', () => {
	it('draws four: the three captured metrics and the counted one', () => {
		const drawn = tiles({});
		expect(drawn).toHaveLength(4);
		expect(drawn.map((tile) => tile.key)).toEqual([
			'sales_count',
			'earnings',
			'resource_views',
			'live_listings'
		]);
	});

	it('marks the counted tile as counted and the reported ones as not', () => {
		const drawn = tiles({});
		expect(drawn.slice(0, 3).every((tile) => tile.counted)).toBe(false);
		expect(drawn[3].counted).toBe(true);
		expect(drawn[3].tag).toBe('from your Resources');
	});

	it('never writes a nil where a marketplace reported nothing', () => {
		for (const tile of tiles({ scope: 'tes' }).slice(0, 3)) {
			expect(tile.value).toBe('—');
			expect(tile.figure).toBe(false);
			expect(tile.tag).toBe('not reported');
			expect(tile.sub).toBe("TES doesn't share figures");
		}
	});

	// The one substitution the whole page is held to. Asserted over every
	// silence rather than over one, because a zero slipped in for any of them
	// is a figure the seller has no reason to distrust.
	it('draws an em dash, never a nil, for every silence there is', () => {
		const silences = [
			tiles({ scope: 'tes' }),
			tiles({ summary: 'failed' }),
			tiles({ summary: 'pending' }),
			tiles({ figures: threeFigures(undefined, null) })
		];
		for (const drawn of silences) {
			for (const tile of drawn.slice(0, 3)) {
				expect(tile.value).toBe('—');
				expect(tile.figure).toBe(false);
				expect(tile.value).not.toBe('0');
			}
		}
	});

	it('says a real nil is a figure, so a captured zero is not mistaken for silence', () => {
		const tile = tiles({ figures: threeFigures(0, HOUR) })[0];
		expect(tile.value).toBe('0');
		expect(tile.figure).toBe(true);
	});

	it('says the read failed rather than that nothing was captured', () => {
		for (const tile of tiles({ summary: 'failed' }).slice(0, 3)) {
			expect(tile.value).toBe('—');
			expect(tile.figure).toBe(false);
			expect(tile.sub).toBe('could not load');
		}
	});

	it('says the read is in flight rather than that nothing was captured', () => {
		for (const tile of tiles({ summary: 'pending' }).slice(0, 3)) {
			expect(tile.value).toBe('—');
			expect(tile.figure).toBe(false);
			expect(tile.sub).toBe('loading…');
		}
	});

	it('says nothing was captured only when the read landed and held none', () => {
		for (const tile of tiles({ figures: threeFigures(undefined, null) }).slice(0, 3)) {
			expect(tile.value).toBe('—');
			expect(tile.figure).toBe(false);
			expect(tile.sub).toBe('no figures yet');
		}
	});

	it('lets a marketplace that reports nothing speak before the read state does', () => {
		const tile = tiles({ scope: 'tes', summary: 'failed' })[0];
		expect(tile.sub).toBe("TES doesn't share figures");
	});

	it('dates a reported figure by its own oldest contributor', () => {
		const tile = tiles({ figures: threeFigures(10, HOUR) })[0];
		expect(tile.value).toBe('10');
		expect(tile.sub).toBe('as of 1 h ago');
	});

	it('tags the combined scope so a TPT-only total is not read as every marketplace', () => {
		expect(tiles({ scope: 'all' })[0].tag).toBe('TPT only');
		expect(tiles({ scope: 'tpt' })[0].tag).toBeUndefined();
	});

	it('holds the counted tile back until the catalogue has been read', () => {
		const standing: Standing = { listings: 9, live: 4, drafts: 0, unsent: 0, other: 5 };
		expect(tiles({ catalogue: 'pending', standing })[3]).toMatchObject({
			value: '—',
			figure: false,
			sub: 'counting…'
		});
		expect(tiles({ catalogue: 'failed', standing })[3]).toMatchObject({
			value: '—',
			figure: false,
			sub: 'could not load your resources'
		});
		expect(tiles({ catalogue: 'read', standing })[3]).toMatchObject({
			value: '4',
			figure: true,
			sub: 'of 9 tracked'
		});
	});
});

describe('the header freshness', () => {
	const captured = [
		listing('m1', 'Tpt', { sales_count: 1 }, HOUR),
		listing('m2', 'Tpt', { sales_count: 1 }, 9 * HOUR)
	];

	it('is dated by the oldest capture, so it never reads fresher than the tiles', () => {
		const meta = headerMeta({ scope: 'tpt', listings: captured, summary: 'read', now: 10 * HOUR });
		expect(meta.at).toBe(HOUR);
		expect(meta.value).toBe('9 h ago');
	});

	it('answers as the tiles do for a scope no marketplace reports for', () => {
		const meta = headerMeta({ scope: 'tes', listings: captured, summary: 'read', now: 10 * HOUR });
		expect(meta.value).toBe('not reported');
		expect(meta.at).toBeNull();
	});

	it('states a failed or in-flight read rather than an absence of figures', () => {
		expect(
			headerMeta({ scope: 'tpt', listings: [], summary: 'failed', now: 0 }).value
		).toBe('could not load');
		expect(
			headerMeta({ scope: 'tpt', listings: [], summary: 'pending', now: 0 }).value
		).toBe('loading…');
		expect(headerMeta({ scope: 'tpt', listings: [], summary: 'read', now: 0 }).value).toBe(
			'nothing yet'
		);
	});
});

describe('the chart view', () => {
	it('charts reported figures uncounted, and standings counted', () => {
		const reported = chartView({ scope: 'tpt', rows: [], standing: NOTHING, limit: 8, summary: 'read', catalogue: 'read' });
		expect(reported.counted).toBe(false);
		expect(reported.title).toBe('Most viewed resources');

		const stood = chartView({ scope: 'tes', rows: [], standing: NOTHING, limit: 8, summary: 'read', catalogue: 'read' });
		expect(stood.counted).toBe(true);
		expect(stood.title).toBe('Where your listings stand');
		expect(stood.description).toContain('TES');
	});

	it('names the cap only where the cap actually bites', () => {
		const rows = resourceRows(
			[1, 2, 3].map((n) => listing(`m${n}`, 'Tpt', { resource_views: n })),
			new Map(),
			'tpt'
		);
		expect(chartView({ scope: 'tpt', rows, standing: NOTHING, limit: 8, summary: 'read', catalogue: 'read' }).description).not.toContain(
			'8'
		);
		expect(chartView({ scope: 'tpt', rows, standing: NOTHING, limit: 3, summary: 'read', catalogue: 'read' }).description).toContain(
			'3'
		);
	});
});

describe('the table panel wording', () => {
	it('promises a ranking only where there is something to rank by', () => {
		expect(tableView('tpt', 'read').description).toContain('best first');
		expect(tableView('all', 'read').description).toContain('best first');
	});

	it('says why nothing is ranked where no marketplace reports', () => {
		const said = tableView('tes', 'read').description;
		expect(said).not.toContain('best first');
		expect(said).toContain('TES');
	});
});

describe('nesting the portfolio rows', () => {
	it('puts a breakdown under the figure above it, never beside it', () => {
		const groups = groupPortfolioRows(PORTFOLIO_ROWS);
		expect(groups[0].row.key).toBe('live');
		expect(groups[0].under.map((row) => row.key)).toEqual(['livePriced', 'liveFree']);
		expect(groups.every((group) => group.row.breakdown === undefined)).toBe(true);
	});

	it('keeps every row exactly once', () => {
		const groups = groupPortfolioRows(PORTFOLIO_ROWS);
		const flattened = groups.flatMap((group) => [group.row, ...group.under]);
		expect(flattened).toHaveLength(PORTFOLIO_ROWS.length);
		expect(new Set(flattened.map((row) => row.key)).size).toBe(PORTFOLIO_ROWS.length);
	});

	it('does not nest a breakdown that leads the list, having nothing to nest under', () => {
		const groups = groupPortfolioRows([
			{ key: 'livePriced', label: 'a', explanation: 'a', breakdown: true },
			{ key: 'live', label: 'b', explanation: 'b' }
		]);
		expect(groups).toHaveLength(2);
	});
});

describe('the TES membership maps agree', () => {
	// The scope map here and `IS_TES` inside `tes-portfolio` are two total maps
	// over the same union, and the TES tab draws counts from both. Both stop
	// type-checking when the union widens, but nothing stops the two answers
	// diverging, which would put two different totals for the same thing on one
	// screen. Asserted through the exported functions, because `IS_TES` is
	// private to its module.
	it('counts a mapping on the TES tab exactly when the portfolio counts it', () => {
		for (const inventory of INVENTORIES) {
			const one = [mapping('m', inventory)];
			expect(standings(one, 'tes').listings).toBe(tesPortfolio([], one).listings);
		}
	});

	it('agrees on the live count too, not merely on membership', () => {
		for (const inventory of INVENTORIES) {
			const one = [mapping('m', inventory)];
			expect(standings(one, 'tes').live).toBe(tesPortfolio([], one).live);
		}
	});
});

describe('narrowing to a label', () => {
	const products = [
		{ id: 'p1', title: 'Kept', price: null, created_at: 0, updated_at: 0 },
		{ id: 'p2', title: 'Also kept', price: null, created_at: 0, updated_at: 0 }
	];
	const mappings = [
		{ ...mapping('m1', 'Tpt'), product: 'p1' },
		{ ...mapping('m2', 'Tpt'), product: 'p3' },
		{ ...mapping('m3', 'Tes'), product: 'p2' }
	];

	it('keeps only the mappings whose product survived the narrowed read', () => {
		expect(mappingsForProducts(mappings, products).map((one) => one.id)).toEqual(['m1', 'm3']);
	});

	it('keeps only the captured listings belonging to those mappings', () => {
		const captured = [
			listing('m1', 'Tpt', { sales_count: 1 }),
			listing('m2', 'Tpt', { sales_count: 1 })
		];
		expect(
			listingsForMappings(captured, mappingsForProducts(mappings, products)).map(
				(one) => one.mapping
			)
		).toEqual(['m1']);
	});

	it('keeps nothing while the narrowed catalogue is still in flight', () => {
		expect(mappingsForProducts(mappings, [])).toEqual([]);
	});
});


describe('a panel says nothing until its read lands', () => {
	const rows: never[] = [];

	it('drops the chart description on a failed or pending capture read', () => {
		for (const summary of ['failed', 'pending'] as const) {
			const view = chartView({
				scope: 'tpt',
				rows,
				standing: NOTHING,
				limit: 8,
				summary,
				catalogue: 'read'
			});
			expect(view.description).toBe('');
			expect(view.title).toBe('Most viewed resources');
		}
	});

	it('drops the standings description on a failed or pending catalogue read', () => {
		for (const catalogue of ['failed', 'pending'] as const) {
			expect(
				chartView({
					scope: 'tes',
					rows,
					standing: NOTHING,
					limit: 8,
					summary: 'read',
					catalogue
				}).description
			).toBe('');
		}
	});

	it('drops the table description on a failed read, but keeps the reason nothing is ranked', () => {
		expect(tableView('tpt', 'failed').description).toBe('');
		// True whatever the read did, so it stays.
		expect(tableView('tes', 'failed').description).toContain('TES');
	});
});

describe('the contributor count', () => {
	it('is named only where the total is short of the scope', () => {
		const partial = tiles({ figures: threeFigures(10, HOUR, 4) })[0];
		expect(partial.sub).toBe('as of 1 h ago, from 1 of 4 listings');

		const whole = tiles({ figures: threeFigures(10, HOUR, 1) })[0];
		expect(whole.sub).toBe('as of 1 h ago');
	});

	it('is counted over the listings the scope covers, not the whole read', () => {
		const captured = [
			listing('m1', 'Tpt', { sales_count: 1 }),
			listing('m2', 'Tpt', {}),
			listing('m3', 'Tes', { sales_count: 9 })
		];
		const summed = figures(captured, 'tpt')[0];
		expect(summed.from).toBe(1);
		expect(summed.of).toBe(2);
	});
});

describe('the TES coverage is the portfolio map itself', () => {
	it('is the same object, so the two cannot drift apart at all', () => {
		for (const inventory of INVENTORIES) {
			expect(covers('tes', inventory)).toBe(IS_TES[inventory]);
		}
	});
});
