// Admin → Mail's own words and rules: the variables a body may carry, the
// audiences a campaign can go to, the sentences the pages say about both, and
// the email's allow-list. Pure, so every rule tests without a component.

import type {
	MailAudience,
	MailCounts,
	MailDraft,
	MailRecipientStatus,
	MailSegment
} from '$lib/api';
import type { Tone } from '$lib/StatusPill.svelte';
import { reduceHtml, safeHref, type AllowList } from '$lib/rich-text';

export interface MailVariable {
	token: string;
	label: string;
	hint: string;
}

/** The words the server swaps for each recipient's own, in toolbar order. */
export const MAIL_VARIABLES: readonly MailVariable[] = [
	{ token: '@name', label: 'Full name', hint: 'The seller’s full name.' },
	{ token: '@first_name', label: 'First name', hint: 'The seller’s first name.' },
	{ token: '@email', label: 'Email', hint: 'The address the email goes to.' },
	{ token: '@org', label: 'Organisation', hint: 'The name of the seller’s organisation.' },
	{ token: '@plan', label: 'Plan', hint: 'The plan the seller is on.' },
	{ token: '@date', label: 'Date', hint: 'The day it sends, like 29 September 2026.' },
	{
		token: '@link',
		label: 'Button',
		hint: 'On a line of its own, the button below; inside a sentence, a link to the same address.'
	},
	{ token: '@unsubscribe', label: 'Unsubscribe', hint: 'An “unsubscribe” link.' }
];

export interface SegmentChoice {
	value: MailSegment;
	label: string;
	/** The segment as the subject of a sentence. */
	who: string;
}

export const SEGMENTS: readonly SegmentChoice[] = [
	{ value: 'all', label: 'All sellers', who: 'All sellers' },
	{ value: 'free', label: 'Free (Look)', who: 'Sellers on Free (Look)' },
	{ value: 'paid', label: 'Paid', who: 'Sellers on any paid plan' },
	{ value: 'starter', label: 'Starter', who: 'Sellers on Starter' },
	{ value: 'subscriber', label: 'Sync', who: 'Sellers on Sync' },
	{ value: 'studio', label: 'Studio', who: 'Sellers on Studio' }
];

/** Who a campaign goes to, as one sentence. */
export function audienceSummary(audience: MailAudience): string {
	const who = SEGMENTS.find((choice) => choice.value === audience.segment)?.who ?? 'Sellers';
	const admins = audience.exclude_operators ? 'admins left out' : 'admins included';
	const addresses = audience.verified_only ? 'verified addresses only' : 'any address';
	return `${who}, ${admins}, ${addresses}.`;
}

function number(count: number): string {
	return count.toLocaleString('en-GB');
}

/** A number of sellers, in words: "1 seller", "1,280 sellers". */
export function sellers(count: number): string {
	return count === 1 ? '1 seller' : `${number(count)} sellers`;
}

/** The live count under the audience filters. */
export function countLine(count: number): string {
	return count === 0 ? 'Nobody matches these filters.' : `${sellers(count)} will get this.`;
}

/** What a campaign's recipients came to, the states with nobody in them left
 *  out. */
export function countsLine(counts: MailCounts): string {
	if (counts.total === 0) {
		return 'Nobody.';
	}
	const parts = (
		[
			['sent', counts.sent],
			['failed', counts.failed],
			['queued', counts.queued],
			['skipped', counts.skipped]
		] as const
	)
		.filter(([, count]) => count > 0)
		.map(([word, count]) => `${number(count)} ${word}`);
	return `${parts.join(', ')}.`;
}

const STATUS_TONES: Record<MailRecipientStatus, Tone> = {
	sent: 'ok',
	failed: 'bad',
	queued: 'run',
	skipped: 'soon'
};

export function statusTone(status: MailRecipientStatus): Tone {
	return STATUS_TONES[status];
}

/** A picture stored by `POST /v1/admin/mail/images`, or any https address. */
const MAIL_IMAGE = /^\/v1\/mail\/images\/[0-9a-f]{64}$/;

const MAIL_ALLOW: AllowList = {
	headings: true,
	image: (src) => (MAIL_IMAGE.test(src) || /^https:\/\/[^\s"<>]+$/i.test(src) ? src : null)
};

/** An email body as the server keeps it: the description's eight elements
 *  plus `h2`, `h3` and pictures from our own store or an https address. */
export function sanitiseMailHtml(html: string): string {
	return reduceHtml(html, MAIL_ALLOW);
}

/** Whether the body's words carry one variable. `@name` is not found inside
 *  `@names`, nor inside an address like `x@name.com`. */
export function usesVariable(html: string, token: string): boolean {
	const text = html.replace(/<[^>]*>/g, ' ');
	const escaped = token.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
	return new RegExp(`(^|[^\\w@])${escaped}(?![\\w])`).test(text);
}

export const SUBJECT_MAX = 200;

/** The first thing stopping this draft from sending, or null. The server
 *  refuses the same drafts; this says so before the round trip. */
export function validDraft(draft: MailDraft): string | null {
	const subject = draft.subject.trim();
	if (subject === '') {
		return 'Write a subject.';
	}
	if ([...subject].length > SUBJECT_MAX) {
		return `Keep the subject to ${SUBJECT_MAX} characters or fewer.`;
	}
	if (sanitiseMailHtml(draft.body_html) === '') {
		return 'Write the email.';
	}
	const url = draft.link_url?.trim() ?? '';
	if (url !== '' && !/^https:\/\//i.test(safeHref(url) ?? '')) {
		return 'The button’s address has to start with https://.';
	}
	if (url === '' && usesVariable(draft.body_html, '@link')) {
		return 'The email uses @link, so give the button an address.';
	}
	return null;
}

/** The draft as the API takes it: blanks as null, the body reduced. */
export function mailDraft(fields: {
	subject: string;
	body: string;
	linkUrl: string;
	linkLabel: string;
}): MailDraft {
	return {
		subject: fields.subject.trim(),
		body_html: sanitiseMailHtml(fields.body),
		link_url: fields.linkUrl.trim() === '' ? null : fields.linkUrl.trim(),
		link_label: fields.linkLabel.trim() === '' ? null : fields.linkLabel.trim()
	};
}
