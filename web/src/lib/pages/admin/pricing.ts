/**
 * The admin Pricing page's wire shapes and the rules its three forms check
 * before they send.
 *
 * The server checks every rule again — these exist so the button can say
 * why it is disabled rather than the save failing. Wording is the page's;
 * the vocabulary (price keys, `once`/`repeating`) is the server's.
 */

import type { PriceKey } from '$lib/generated/vocab';
import type { Season } from '$lib/site';

export type DiscountKind = 'sale' | 'one_off' | 'code';
export type DiscountState = 'scheduled' | 'open' | 'over' | 'ended';
export type Duration = 'once' | 'repeating';
/** A sale's theme: any of the site themes the Site page offers. */
export type Theme = Season;

export interface DiscountCodeView {
	id: string;
	code: string;
	max_redemptions: number | null;
	stripe_promotion_code_id: string;
	ended: boolean;
}

export interface DiscountView {
	id: string;
	kind: DiscountKind;
	name: string;
	percent_off: number | null;
	amount_off_cents: number | null;
	currency: string;
	duration: Duration;
	duration_months: number | null;
	/** Empty means every plan. */
	price_keys: string[];
	from: string;
	/** Inclusive. */
	until: string;
	state: DiscountState;
	stripe_coupon_id: string;
	/** Absent when Stripe was not asked. */
	in_stripe: boolean | null;
	banner: string | null;
	banner_href: string | null;
	theme: Theme | null;
	codes: DiscountCodeView[];
}

export interface PriceKeyView {
	key: PriceKey;
	list_cents: number;
	plan: boolean;
}

export interface PricingAdminView {
	discounts: DiscountView[];
	price_keys: PriceKeyView[];
	stripe_configured: boolean;
}

export interface TermsBody {
	name: string;
	percent_off?: number;
	amount_off_cents?: number;
	duration: Duration;
	duration_months?: number;
	price_keys: string[];
	from: string;
	until: string;
}

export interface SaleBody {
	name: string;
	percent_off: number;
	from: string;
	until: string;
	banner: string;
	banner_href?: string;
	theme?: Theme;
	duration: Duration;
	duration_months?: number;
}

export interface CodeBody extends TermsBody {
	code: string;
	max_redemptions?: number;
}

/** What a form holds while it is being filled in. Numbers stay as the
 *  input's own `number | null` until sent. */
export interface TermsDraft {
	name: string;
	/** `percent` or `amount`, which of the two figures applies. */
	by: 'percent' | 'amount';
	percent: number | null;
	/** Dollars, as typed; sent as cents. */
	dollars: number | null;
	duration: Duration;
	months: number | null;
	keys: string[];
	from: string;
	until: string;
}

const DATE = /^\d{4}-\d{2}-\d{2}$/;
const CODE = /^[A-Za-z0-9_-]{3,40}$/;

/** Everything the create form holds, across its three kinds. */
export interface PricingForm {
	kind: DiscountKind;
	terms: TermsDraft;
	banner: string;
	bannerHref: string;
	theme: Theme | 'none';
	code: string;
	/** Use limit; null for none. */
	limit: number | null;
}

/** The form's fields that can be wrong, each of which says so beneath itself. */
export type FormField =
	| 'name'
	| 'code'
	| 'percent'
	| 'dollars'
	| 'months'
	| 'from'
	| 'until'
	| 'keys'
	| 'banner'
	| 'limit';

export type FormProblems = Partial<Record<FormField, string>>;

/** What is wrong with the form, field by field; empty when it can be saved.
 *  `today` is `YYYY-MM-DD` in UTC, the day the server compares windows
 *  against. Only the fields the current kind shows are checked. */
export function formProblems(form: PricingForm, today: string): FormProblems {
	const { kind, terms } = form;
	const problems: FormProblems = {};
	const name = terms.name.trim();
	if (name.length === 0 || name.length > 80) {
		problems.name = 'Give it a name of up to 80 characters.';
	}
	if (kind === 'code' && !CODE.test(form.code.trim())) {
		problems.code = 'A code is 3 to 40 letters, digits, dashes or underscores.';
	}
	if (terms.by === 'percent' || kind === 'sale') {
		if (terms.percent === null || !Number.isInteger(terms.percent)) {
			problems.percent = 'Enter a whole percentage.';
		} else if (terms.percent < 1 || terms.percent > 100) {
			problems.percent = 'A percentage off is between 1 and 100.';
		}
	} else if (terms.dollars === null || !(terms.dollars > 0)) {
		problems.dollars = 'Enter an amount off above zero.';
	}
	if (
		terms.duration === 'repeating' &&
		(terms.months === null ||
			!Number.isInteger(terms.months) ||
			terms.months < 1 ||
			terms.months > 36)
	) {
		problems.months = 'Enter a number of months between 1 and 36.';
	}
	if (!DATE.test(terms.from)) {
		problems.from = 'Choose a first day.';
	}
	// ISO dates compare as strings.
	if (!DATE.test(terms.until)) {
		problems.until = 'Choose a last day.';
	} else if (DATE.test(terms.from) && terms.until < terms.from) {
		problems.until = 'The last day comes on or after the first day.';
	} else if (terms.until < today) {
		problems.until = 'That window has already closed.';
	}
	if (kind === 'one_off' && terms.keys.length === 0) {
		problems.keys = 'Pick at least one price.';
	}
	if (kind === 'sale') {
		const words = form.banner.trim();
		if (words.length === 0 || words.length > 140) {
			problems.banner = 'Write a banner of up to 140 characters.';
		}
	}
	if (kind === 'code' && form.limit !== null && (!Number.isInteger(form.limit) || form.limit < 1)) {
		problems.limit = 'A use limit is at least 1, or empty for none.';
	}
	return problems;
}

const SHORT_MONTHS = [
	'Jan',
	'Feb',
	'Mar',
	'Apr',
	'May',
	'Jun',
	'Jul',
	'Aug',
	'Sep',
	'Oct',
	'Nov',
	'Dec'
];

/** A window of UTC days as a person says it: "1–31 Oct", "28 Oct – 3 Nov",
 *  and the years only where the two days fall in different ones. */
export function windowLabel(from: string, until: string): string {
	if (!DATE.test(from) || !DATE.test(until)) {
		return 'no dates yet';
	}
	const [fromYear, fromMonth, fromDay] = from.split('-').map(Number);
	const [untilYear, untilMonth, untilDay] = until.split('-').map(Number);
	const month = (index: number) => SHORT_MONTHS[index - 1];
	if (fromYear !== untilYear) {
		return `${fromDay} ${month(fromMonth)} ${fromYear} – ${untilDay} ${month(untilMonth)} ${untilYear}`;
	}
	if (fromMonth !== untilMonth) {
		return `${fromDay} ${month(fromMonth)} – ${untilDay} ${month(untilMonth)}`;
	}
	return fromDay === untilDay
		? `${fromDay} ${month(fromMonth)}`
		: `${fromDay}–${untilDay} ${month(fromMonth)}`;
}

/** The form in one line, as it will run: "25% off every plan, 1–31 Oct,
 *  banner “Halloween sale”". A figure not typed yet reads as an ellipsis
 *  rather than a zero, which would read as a decision. */
export function formPreview(form: PricingForm): string {
	const { kind, terms } = form;
	const byPercent = kind === 'sale' || terms.by === 'percent';
	const figure = byPercent
		? terms.percent === null
			? '…% off'
			: `${terms.percent}% off`
		: terms.dollars === null
			? '$… off'
			: discountFigure({ percent_off: null, amount_off_cents: Math.round(terms.dollars * 100) });
	const reach =
		kind === 'one_off' && terms.keys.length === 0 ? 'the prices you pick' : reachLabel(terms.keys);
	const parts = [
		kind === 'code' ? `${form.code.trim() || 'CODE'}: ${figure} ${reach}` : `${figure} ${reach}`
	];
	if (terms.duration === 'repeating') {
		parts.push(
			terms.months === null
				? 'first … months'
				: durationLabel({ duration: 'repeating', duration_months: terms.months })
		);
	}
	parts.push(windowLabel(terms.from, terms.until));
	if (kind === 'sale') {
		const words = form.banner.trim();
		parts.push(words === '' ? 'no banner yet' : `banner “${words}”`);
	}
	if (kind === 'code') {
		parts.push(form.limit === null ? 'no use limit' : `up to ${form.limit} uses`);
	}
	return parts.join(', ');
}

/** One row of the Codes table: a typed code and the discount behind it. */
export interface CodeRow {
	id: string;
	/** The discount the code belongs to, which is what End ends. */
	discountId: string;
	code: string;
	name: string;
	off: string;
	window: string;
	uses: string;
	state: DiscountState;
	/** Whether End applies: the discount is on or ahead, and this code was
	 *  not ended on its own. */
	live: boolean;
}

/** Every typed code across the code discounts, soonest-running first. */
export function codeRows(discounts: readonly DiscountView[]): CodeRow[] {
	return discounts
		.filter((row) => row.kind === 'code')
		.flatMap((row) =>
			row.codes.map((typed) => {
				const state: DiscountState = typed.ended && row.state !== 'over' ? 'ended' : row.state;
				return {
					id: typed.id,
					discountId: row.id,
					code: typed.code,
					name: row.name,
					off: `${discountFigure(row)}, ${durationLabel(row)}`,
					window: windowLabel(row.from, row.until),
					uses: typed.max_redemptions === null ? 'No limit' : `Up to ${typed.max_redemptions}`,
					state,
					live: state === 'open' || state === 'scheduled'
				};
			})
		)
		.sort((one, two) => Number(two.live) - Number(one.live));
}

/** The terms as the server reads them. */
export function termsBody(draft: TermsDraft): TermsBody {
	return {
		name: draft.name.trim(),
		...(draft.by === 'percent'
			? { percent_off: draft.percent ?? 0 }
			: { amount_off_cents: Math.round((draft.dollars ?? 0) * 100) }),
		duration: draft.duration,
		...(draft.duration === 'repeating' ? { duration_months: draft.months ?? 0 } : {}),
		price_keys: draft.keys,
		from: draft.from,
		until: draft.until
	};
}

/** "25% off" or "$10 off", the figure a row leads with. */
export function discountFigure(
	row: Pick<DiscountView, 'percent_off' | 'amount_off_cents'>
): string {
	if (row.percent_off !== null) {
		return `${row.percent_off}% off`;
	}
	const cents = row.amount_off_cents ?? 0;
	return `${cents % 100 === 0 ? `$${cents / 100}` : `$${(cents / 100).toFixed(2)}`} off`;
}

/** "first payment", or "first 3 months". */
export function durationLabel(row: Pick<DiscountView, 'duration' | 'duration_months'>): string {
	if (row.duration === 'once') {
		return 'first payment';
	}
	const months = row.duration_months ?? 0;
	return months === 1 ? 'first month' : `first ${months} months`;
}

/** Which prices a row reaches, in words. */
export function reachLabel(keys: readonly string[]): string {
	return keys.length === 0 ? 'every plan' : keys.join(', ');
}

export const STATE_LABEL: Record<DiscountState, string> = {
	scheduled: 'Scheduled',
	open: 'On now',
	over: 'Finished',
	ended: 'Ended early'
};
