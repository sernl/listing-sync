/**
 * The admin Abuse page's wire shapes and the little it decides on them: the
 * words each kind is shown as, the tone of an organisation's standing, the
 * list's query string, what an action sends, and the row an action redraws.
 *
 * The server owns every rule; the ban reason is checked here only so the
 * panel can say what is missing before it asks.
 */

import type { Tone } from '$lib/StatusPill.svelte';

// ------------------------------------------------------------- vocabulary

export type SignalKind =
	| 'shop_digest'
	| 'device_fingerprint'
	| 'ip'
	| 'email_domain'
	| 'payment_fingerprint'
	| 'user_agent_hash';

export type FlagKind =
	| 'shared_shop'
	| 'shared_device'
	| 'shared_payment'
	| 'signup_burst'
	| 'disposable_email'
	| 'quick_unlink';

/** An organisation's standing, and what an action leaves it at. */
export type AbuseAction = 'none' | 'warn' | 'limit' | 'ban';

/** What an operator presses; `dismiss` leaves the standing at `none`. */
export type OperatorAction = 'dismiss' | 'warn' | 'limit' | 'ban';

export type AbuseState = 'open' | 'all';

/** How the server read the search. */
export type AbuseQueryKind = 'none' | 'email' | 'ip' | 'shop' | 'hash' | 'name';

// ------------------------------------------------------------- wire shapes

export interface AbuseCounters {
	open_orgs: number;
	open_flags: number;
	warned: number;
	limited: number;
	banned: number;
	banned_identities: number;
	signals: number;
}

/** `GET /v1/admin/abuse/flags?state=&q=`. */
export interface AbuseFlagsView {
	counters: AbuseCounters;
	query: AbuseQueryKind;
	/** Highest score first, then newest flag. */
	orgs: AbuseOrgRow[];
}

export interface AbuseOrgRow {
	org: string;
	name: string;
	slug: string | null;
	/** The sum of its open flags' scores; 0 when none is open. */
	score: number;
	kinds: FlagKind[];
	/** Kinds it shares with at least one other organisation. */
	signal_kinds: SignalKind[];
	linked_orgs: number;
	standing: AbuseAction;
	open_flags: number;
	last_flagged_at: number;
	/** Its newest flag, the one an action is sent against. */
	flag: string;
}

/** `GET /v1/admin/abuse/orgs/{org}`, and every action's answer. */
export interface AbuseOrgView {
	org: string;
	name: string;
	slug: string | null;
	created_at: number;
	standing: AbuseAction;
	/** Newest first. */
	flags: AbuseFlagView[];
	signals: AbuseSignalView[];
	linked: LinkedOrgView[];
}

export interface AbuseFlagView {
	id: string;
	kind: FlagKind;
	score: number;
	reason: string;
	created_at: number;
	resolved_at: number | null;
	/** The operator's user id. */
	resolved_by: string | null;
	action: AbuseAction;
	action_reason: string | null;
	mail_sent_at: number | null;
	mail_error: string | null;
}

export interface AbuseSignalView {
	kind: SignalKind;
	/** The first 12 hex characters of the keyed hash, for display only. */
	value: string;
	first_seen: number;
	last_seen: number;
	/** Other organisations holding the same value. */
	shared_with: number;
}

export interface LinkedOrgView {
	org: string;
	name: string;
	standing: AbuseAction;
	shared: SignalKind[];
}

/** The body every action sends. */
export interface AbuseActionBody {
	reason: string | null;
}

// ------------------------------------------------------------------- words

export const FLAG_KIND_LABEL: Record<FlagKind, string> = {
	shared_shop: 'Same shop',
	shared_device: 'Same device',
	shared_payment: 'Same card',
	signup_burst: 'Sign-up burst',
	disposable_email: 'Throwaway email',
	quick_unlink: 'Quick unlink'
};

export const SIGNAL_KIND_LABEL: Record<SignalKind, string> = {
	shop_digest: 'Shop',
	device_fingerprint: 'Device',
	ip: 'IP address',
	email_domain: 'Email domain',
	payment_fingerprint: 'Card',
	user_agent_hash: 'Browser'
};

export const STANDING_LABEL: Record<AbuseAction, string> = {
	none: 'No action',
	warn: 'Warned',
	limit: 'Limited',
	ban: 'Banned'
};

export const STANDING_TONE: Record<AbuseAction, Tone> = {
	none: 'soon',
	warn: 'warn',
	limit: 'bad',
	ban: 'bad'
};

/** What a resolved flag's action is shown as. */
export const RESOLUTION_LABEL: Record<AbuseAction, string> = {
	none: 'Dismissed',
	warn: 'Warned',
	limit: 'Limited',
	ban: 'Banned'
};

export const ACTIONS: readonly OperatorAction[] = ['dismiss', 'warn', 'limit', 'ban'];

export const ACTION_LABEL: Record<OperatorAction, string> = {
	dismiss: 'Dismiss',
	warn: 'Warn',
	limit: 'Limit',
	ban: 'Ban'
};

/** The toast after an action lands. */
export const ACTION_DONE: Record<OperatorAction, string> = {
	dismiss: 'Flags dismissed.',
	warn: 'Warned. The email is on its way.',
	limit: 'Account limited.',
	ban: 'Banned. Everyone in it is signed out.'
};

export const QUERY_LABEL: Record<AbuseQueryKind, string> = {
	none: '',
	email: 'Searched by email address',
	ip: 'Searched by IP address',
	shop: 'Searched by shop',
	hash: 'Searched by signal value',
	name: 'Searched by name'
};

/** The one sentence the ban confirmation leads with. */
export function banSentence(name: string): string {
	return `Banning signs everyone in ${name} out, suspends the account and refuses its email, shops, devices and cards for 24 months.`;
}

// ------------------------------------------------------------------ the list

/** The list's query string: `state` always, `q` only when it says something. */
export function flagsQuery(state: AbuseState, q: string): string {
	const query = new URLSearchParams({ state });
	const search = q.trim();
	if (search.length > 0) query.set('q', search);
	return query.toString();
}

/** Its open flags' kinds, once each, in the order they were raised. */
function openKinds(view: AbuseOrgView): FlagKind[] {
	const kinds: FlagKind[] = [];
	for (const flag of [...view.flags].reverse()) {
		if (flag.resolved_at === null && !kinds.includes(flag.kind)) kinds.push(flag.kind);
	}
	return kinds;
}

/** The list's row for an organisation, redrawn from an action's answer so the
 *  table agrees with the sheet before the list is read again. */
export function patchRow(row: AbuseOrgRow, view: AbuseOrgView): AbuseOrgRow {
	if (row.org !== view.org) return row;
	const open = view.flags.filter((flag) => flag.resolved_at === null);
	return {
		...row,
		standing: view.standing,
		open_flags: open.length,
		score: open.reduce((sum, flag) => sum + flag.score, 0),
		kinds: open.length > 0 ? openKinds(view) : row.kinds
	};
}

/** The same, over a whole list answer. */
export function patchFlags(list: AbuseFlagsView, view: AbuseOrgView): AbuseFlagsView {
	return { ...list, orgs: list.orgs.map((row) => patchRow(row, view)) };
}

// ------------------------------------------------------------------ the sheet

/** The flag an action from the sheet is sent against: its newest, as the
 *  list's row names it; `null` when the organisation has never been flagged. */
export function actionFlag(view: AbuseOrgView): string | null {
	return view.flags[0]?.id ?? null;
}

export const REASON_MAX = 500;

/** Why the reason cannot be sent with this action, or `null` when it can. */
export function reasonProblem(action: OperatorAction, reason: string): string | null {
	const text = reason.trim();
	if (text.length > REASON_MAX) return `Keep it under ${REASON_MAX} characters.`;
	if (action === 'ban' && text.length === 0) return 'Say why. It stays with the ban.';
	return null;
}

/** What an action sends: the reason trimmed, or `null` when there is none. */
export function actionBody(reason: string): AbuseActionBody {
	const text = reason.trim();
	return { reason: text.length > 0 ? text : null };
}

/** Whether an action would change nothing: pressing the standing it already has. */
export function actionIsCurrent(action: OperatorAction, view: AbuseOrgView): boolean {
	const open = view.flags.some((flag) => flag.resolved_at === null);
	const standing: AbuseAction = action === 'dismiss' ? 'none' : action;
	return !open && view.standing === standing;
}
