/**
 * The admin Pricing page's wire shapes and the rules its three forms check
 * before they send.
 *
 * The server checks every rule again — these exist so the button can say
 * why it is disabled rather than the save failing. Wording is the page's;
 * the vocabulary (price keys, `once`/`repeating`) is the server's.
 */

import type { PriceKey } from '$lib/generated/vocab';

export type DiscountKind = 'sale' | 'one_off' | 'code';
export type DiscountState = 'scheduled' | 'open' | 'over' | 'ended';
export type Duration = 'once' | 'repeating';
export type Theme = 'halloween' | 'christmas';

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

/** Why a draft's terms cannot be saved, or null when they can. `today` is
 *  `YYYY-MM-DD` in UTC, the day the server compares windows against. */
export function termsRefusal(draft: TermsDraft, today: string): string | null {
	const name = draft.name.trim();
	if (name.length === 0 || name.length > 80) {
		return 'Give it a name of up to 80 characters.';
	}
	if (draft.by === 'percent') {
		if (draft.percent === null || !Number.isInteger(draft.percent)) {
			return 'Enter a whole percentage.';
		}
		if (draft.percent < 1 || draft.percent > 100) {
			return 'A percentage off is between 1 and 100.';
		}
	} else if (draft.dollars === null || !(draft.dollars > 0)) {
		return 'Enter an amount off above zero.';
	}
	if (
		draft.duration === 'repeating' &&
		(draft.months === null ||
			!Number.isInteger(draft.months) ||
			draft.months < 1 ||
			draft.months > 36)
	) {
		return 'Enter a number of months between 1 and 36.';
	}
	if (!DATE.test(draft.from) || !DATE.test(draft.until)) {
		return 'Choose a first and a last day.';
	}
	// ISO dates compare as strings.
	if (draft.until < draft.from) {
		return 'The last day comes on or after the first day.';
	}
	if (draft.until < today) {
		return 'That window has already closed.';
	}
	return null;
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

const CODE = /^[A-Za-z0-9_-]{3,40}$/;

/** Why a code cannot be used as typed, or null. */
export function codeRefusal(code: string): string | null {
	return CODE.test(code.trim())
		? null
		: 'A code is 3 to 40 letters, digits, dashes or underscores.';
}

/** "25% off" or "$10 off", the figure a row leads with. */
export function discountFigure(row: Pick<DiscountView, 'percent_off' | 'amount_off_cents'>): string {
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
