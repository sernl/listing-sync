// The admin Site analytics page's model: the wire shape of
// `GET /v1/admin/analytics/site` (crates/tam-api/src/admin_analytics.rs) and
// everything the page derives from it — the conversion rate, the plain-words
// labels for PostHog's raw values, and the visitors-per-day line's geometry.
//
// Pure, so the page draws what these functions answer and the tests pin them.

import { ApiFailure } from '$lib/http';
import type { Bar } from '$lib/pages/analytics/model';

export type SiteRange = '7d' | '30d' | '90d';

export interface DayPoint {
	/** `YYYY-MM-DD`, a New Zealand day. */
	day: string;
	pageviews: number;
	visitors: number;
}

export interface SiteRow {
	/** `null` where PostHog recorded nothing for the property. */
	label: string | null;
	/** The country beside a city; `null` elsewhere. */
	detail: string | null;
	visitors: number;
	pageviews: number;
}

export interface SiteTotals {
	pageviews: number;
	visitors: number;
	signups: number;
	cta_clicks: number;
}

export interface SiteAnalyticsView {
	range: SiteRange;
	from: string;
	to: string;
	site_host: string | null;
	totals: SiteTotals;
	days: DayPoint[];
	pages: SiteRow[];
	referrers: SiteRow[];
	utm_sources: SiteRow[];
	countries: SiteRow[];
	cities: SiteRow[];
	devices: SiteRow[];
	browsers: SiteRow[];
	systems: SiteRow[];
	fetched_at: number;
}

export const RANGES: readonly { id: SiteRange; label: string }[] = [
	{ id: '7d', label: '7 days' },
	{ id: '30d', label: '30 days' },
	{ id: '90d', label: '90 days' }
];

export const DEFAULT_RANGE: SiteRange = '7d';

/** The range a `?range=` value names, or the default for anything else. */
export function parseRange(raw: string | null | undefined): SiteRange {
	return RANGES.find((range) => range.id === raw)?.id ?? DEFAULT_RANGE;
}

/** Why the page has no figures: the server has no PostHog key (503), or it
 *  asked and PostHog did not answer, or the request itself failed. */
export type Failure = 'unconfigured' | 'upstream' | 'other';

export function failureOf(error: unknown): Failure {
	if (error instanceof ApiFailure) {
		if (error.status === 503) return 'unconfigured';
		if (error.status === 502) return 'upstream';
	}
	return 'other';
}

/** Signups per visitor over the range, or `null` with no visitors to divide
 *  by. Signups are counted by the server and visitors by the landing, so the
 *  ratio can pass 1 on a quiet week; it is shown as it is. */
export function conversionRate(totals: SiteTotals): number | null {
	return totals.visitors > 0 ? totals.signups / totals.visitors : null;
}

export function formatPercent(rate: number | null): string {
	if (rate === null) return '—';
	const percent = rate * 100;
	return `${percent < 10 ? percent.toFixed(1) : Math.round(percent).toString()}%`;
}

const COUNT = new Intl.NumberFormat('en-NZ');

export function formatCount(value: number): string {
	return COUNT.format(value);
}

/** A breakdown value PostHog left empty. */
export const UNKNOWN = 'Unknown';

export function rowLabel(label: string | null): string {
	return label ?? UNKNOWN;
}

/** A path as a teacher would say it. */
export function pageLabel(label: string | null): string {
	if (label === null) return UNKNOWN;
	return label === '/' ? 'Home page (/)' : label;
}

/** PostHog writes `$direct` for a visit with no referring site: a typed
 *  address, a bookmark, or an app that sends none. */
export function referrerLabel(label: string | null): string {
	if (label === null) return UNKNOWN;
	return label === '$direct' ? 'Direct or bookmarked' : label;
}

export function cityLabel(row: SiteRow): string {
	const city = rowLabel(row.label);
	return row.detail === null ? city : `${city}, ${row.detail}`;
}

/** Breakdown rows as `MetricBars` draws them, longest first as PostHog
 *  ordered them, each bar's length a share of the most visitors. */
export function bars(rows: readonly SiteRow[], label: (row: SiteRow) => string): Bar[] {
	const most = rows.reduce((largest, row) => Math.max(largest, row.visitors), 0);
	return rows.map((row) => {
		const text = label(row);
		return {
			label: text,
			title: `${text}: ${row.visitors} visitors, ${row.pageviews} page views`,
			value: row.visitors,
			share: most > 0 ? row.visitors / most : 0
		};
	});
}

const DAY = new Intl.DateTimeFormat('en-NZ', {
	weekday: 'short',
	day: 'numeric',
	month: 'short',
	timeZone: 'UTC'
});

const SHORT_DAY = new Intl.DateTimeFormat('en-NZ', {
	day: 'numeric',
	month: 'short',
	timeZone: 'UTC'
});

/** "Sat, 3 Oct" for a New Zealand day. The day is already New Zealand's, so
 *  it is formatted as the calendar date it names rather than converted. */
export function dayLabel(day: string): string {
	return DAY.format(new Date(`${day}T00:00:00Z`));
}

/** "3 Oct", for the chart's axis. */
export function shortDay(day: string): string {
	return SHORT_DAY.format(new Date(`${day}T00:00:00Z`));
}

/** "28 Sept – 4 Oct", the range the figures cover. */
export function windowLabel(view: Pick<SiteAnalyticsView, 'from' | 'to'>): string {
	return `${shortDay(view.from)} – ${shortDay(view.to)}`;
}

const TIME = new Intl.DateTimeFormat('en-NZ', {
	hour: 'numeric',
	minute: '2-digit',
	timeZone: 'Pacific/Auckland'
});

export function updatedLabel(fetchedAt: number): string {
	return `Updated ${TIME.format(new Date(fetchedAt))}`;
}

/** The smallest round number at or above `value` the axis can end on: 1, 2,
 *  4, 6 or 8 times a power of ten, so its half is a whole number too, and at
 *  least 4 so a quiet week still has gridlines worth drawing. */
export function niceMax(value: number): number {
	const floor = Math.max(value, 4);
	const power = 10 ** Math.floor(Math.log10(floor));
	for (const step of [1, 2, 4, 6, 8, 10]) {
		if (step * power >= floor) return step * power;
	}
	return 10 * power;
}

export interface LinePoint {
	x: number;
	y: number;
	day: string;
	visitors: number;
	pageviews: number;
}

export interface LineGeometry {
	/** SVG path for the visitors line. */
	line: string;
	/** The same line closed down to the baseline, for the tint beneath it. */
	area: string;
	points: LinePoint[];
	/** Gridline values, baseline first, ending at the axis maximum. */
	ticks: { value: number; y: number }[];
	/** About five days to name under the axis, evenly spaced, the last day
	 *  always among them. */
	labels: { x: number; day: string }[];
}

export interface Frame {
	width: number;
	height: number;
	/** Room for the tick labels on the left and the day labels below. */
	left: number;
	bottom: number;
	top: number;
	right: number;
}

const LABELS_ABOUT = 5;

function round(value: number): number {
	return Math.round(value * 10) / 10;
}

/** The visitors-per-day line laid out in `frame`. One day draws a single
 *  point in the middle; no days draws nothing. */
export function lineGeometry(days: readonly DayPoint[], frame: Frame): LineGeometry {
	const plotWidth = Math.max(frame.width - frame.left - frame.right, 1);
	const plotHeight = Math.max(frame.height - frame.top - frame.bottom, 1);
	const baseline = frame.top + plotHeight;
	const max = niceMax(days.reduce((most, day) => Math.max(most, day.visitors), 0));
	const step = days.length > 1 ? plotWidth / (days.length - 1) : 0;
	const points = days.map((day, index) => ({
		x: round(frame.left + (days.length > 1 ? index * step : plotWidth / 2)),
		y: round(baseline - (day.visitors / max) * plotHeight),
		day: day.day,
		visitors: day.visitors,
		pageviews: day.pageviews
	}));
	const line = points
		.map((point, index) => `${index === 0 ? 'M' : 'L'}${point.x} ${point.y}`)
		.join(' ');
	const first = points[0];
	const last = points[points.length - 1];
	const area =
		first === undefined || last === undefined
			? ''
			: `${line} L${last.x} ${round(baseline)} L${first.x} ${round(baseline)} Z`;
	const ticks = [0, 0.5, 1].map((fraction) => ({
		value: max * fraction,
		y: round(baseline - fraction * plotHeight)
	}));
	// Every `every`th day from the first, skipping one that would crowd the
	// last day, which is always named.
	const every = Math.max(1, Math.ceil(points.length / LABELS_ABOUT));
	const named = points.filter(
		(_, index) => index % every === 0 && points.length - 1 - index >= every / 2
	);
	if (last !== undefined) named.push(last);
	const labels = named.map((point) => ({ x: point.x, day: point.day }));
	return { line, area, points, ticks, labels };
}
