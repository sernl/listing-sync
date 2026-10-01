/** The company's calendar. A sale's days and a theme's days are days in New
 *  Zealand, where Teachouse operates; the server reads them in the same zone
 *  (`SITE_TIMEZONE` in `crates/tam-api/src/time.rs`). */
export const SITE_TIMEZONE = 'Pacific/Auckland';

/** The one sentence the admin Pricing and Site pages say about it. */
export const DAYS_NOTE = 'Days follow New Zealand time.';

const DAY = new Intl.DateTimeFormat('en-CA', {
	timeZone: SITE_TIMEZONE,
	year: 'numeric',
	month: '2-digit',
	day: '2-digit'
});

/** Today in New Zealand as `YYYY-MM-DD`, the day the server compares a
 *  window against. */
export function siteToday(now: Date = new Date()): string {
	const parts = DAY.formatToParts(now);
	const part = (type: Intl.DateTimeFormatPartTypes) =>
		parts.find((entry) => entry.type === type)?.value ?? '';
	return `${part('year')}-${part('month')}-${part('day')}`;
}
