// The admin pages' view-model: the joins, filters and colour codes the
// templates draw from, kept here so each can be tested without a page.

import type {
	AdminUserView,
	FailedWriteView,
	ImpersonationView,
	OrgSummaryView,
	SyncHealthView
} from '$lib/api';
import type { GateWindow } from '$lib/drain';
import { PLANS } from '$lib/generated/plans';
import type { Plan } from '$lib/generated/vocab';
import type { Tone } from '$lib/StatusPill.svelte';

/** The plan's own name where the price table carries it, and the wire word
 *  otherwise: a grant written before a plan was renamed still names a plan,
 *  and printing nothing there would hide which. */
export function planName(plan: Plan): string {
	return PLANS.find((row) => row.id === plan)?.name ?? plan;
}

/** Free is grey, anything paid is green: the one distinction the list is
 *  scanned for. */
export function planTone(plan: Plan): Tone {
	return plan === 'free' ? 'soon' : 'ok';
}

// --- organisations -----------------------------------------------------------

/** One organisation as the list draws it: the summary the orgs endpoint
 *  serves, joined to the plan and last sign-in the users endpoint carries.
 *  `plan` and `lastSeen` are null where no user of the organisation is on the
 *  users answer, which is "not known" rather than "Free" or "never". */
export interface OrgRow extends OrgSummaryView {
	plan: Plan | null;
	lastSeen: number | null;
}

export function orgRows(
	orgs: readonly OrgSummaryView[],
	users: readonly AdminUserView[]
): OrgRow[] {
	const plan = new Map<string, Plan>();
	const seen = new Map<string, number>();
	for (const user of users) {
		const org = user.organisation.org;
		plan.set(org, user.plan);
		const at = user.last_sign_in_at;
		if (at !== undefined && at > (seen.get(org) ?? -Infinity)) {
			seen.set(org, at);
		}
	}
	return orgs.map((org) => ({
		...org,
		plan: plan.get(org.org) ?? null,
		lastSeen: seen.get(org.org) ?? null
	}));
}

export type OrgFilter = 'all' | 'paid' | 'free' | 'no-shop';

export const ORG_FILTERS: readonly { id: OrgFilter; label: string }[] = [
	{ id: 'all', label: 'All' },
	{ id: 'paid', label: 'Paid' },
	{ id: 'free', label: 'Free' },
	{ id: 'no-shop', label: 'No shop' }
];

/** Whether a row belongs under a filter chip. An organisation whose plan is
 *  not known is under neither Paid nor Free. */
export function orgInFilter(row: OrgRow, filter: OrgFilter): boolean {
	switch (filter) {
		case 'all':
			return true;
		case 'paid':
			return row.plan !== null && row.plan !== 'free';
		case 'free':
			return row.plan === 'free';
		case 'no-shop':
			return row.connections === 0;
	}
}

/** Whether a row matches the search box: its name, slug or id, ignoring
 *  case. An empty box matches everything. */
export function orgMatches(row: OrgRow, query: string): boolean {
	const needle = query.trim().toLocaleLowerCase();
	if (needle.length === 0) {
		return true;
	}
	return [row.name, row.slug ?? '', row.org].some((field) =>
		field.toLocaleLowerCase().includes(needle)
	);
}

// --- sync health --------------------------------------------------------------

export type HealthGroup = 'moving' | 'held' | 'outcome';

export interface HealthRow {
	key: keyof SyncHealthView;
	label: string;
	/** What the state means, for the column's Explain. */
	note: string;
	group: HealthGroup;
	tone: Tone;
}

/** The stored state vocabulary, uncollapsed, then the settled outcomes. A
 *  seller's job page folds leased, running and verifying into one figure and
 *  both park states into another; here parked-live against parked-cold is the
 *  distinction an operator opened the page to find. */
export const HEALTH_ROWS: readonly HealthRow[] = [
	{ key: 'queued', label: 'Queued', note: 'Waiting for a worker to take them.', group: 'moving', tone: 'run' },
	{ key: 'leased', label: 'Leased', note: 'Claimed by a worker, not yet started.', group: 'moving', tone: 'run' },
	{ key: 'running', label: 'Running', note: 'A write is in progress.', group: 'moving', tone: 'run' },
	{ key: 'verifying', label: 'Verifying', note: 'Written, reading back to confirm.', group: 'moving', tone: 'run' },
	{ key: 'blocked', label: 'Blocked', note: 'Waiting on something else to settle first.', group: 'held', tone: 'warn' },
	{ key: 'parked_live', label: 'Parked (live)', note: 'Held with the connection still usable.', group: 'held', tone: 'warn' },
	{ key: 'parked_cold', label: 'Parked (cold)', note: 'Held with nothing usable stored.', group: 'held', tone: 'warn' },
	{ key: 'succeeded', label: 'Succeeded', note: 'Settled and written as asked.', group: 'outcome', tone: 'ok' },
	{ key: 'degraded', label: 'Degraded', note: 'Settled, but not everything was written.', group: 'outcome', tone: 'run' },
	{ key: 'failed', label: 'Failed', note: 'Settled with a failure.', group: 'outcome', tone: 'bad' },
	{ key: 'ambiguous', label: 'Ambiguous', note: 'Settled without knowing whether the write landed.', group: 'outcome', tone: 'bad' },
	{ key: 'skipped', label: 'Skipped', note: 'Settled with nothing to write.', group: 'outcome', tone: 'soon' },
	{ key: 'outcome_blocked', label: 'Blocked (settled)', note: 'Settled because something refused it.', group: 'outcome', tone: 'soon' }
];

export const HEALTH_FILTERS: readonly { id: HealthGroup | 'all'; label: string }[] = [
	{ id: 'all', label: 'All' },
	{ id: 'moving', label: 'Moving' },
	{ id: 'held', label: 'Held' },
	{ id: 'outcome', label: 'Settled' }
];

// --- failed writes -------------------------------------------------------------

export type WriteKind = 'failed' | 'stranded';

/** A write with a failure code failed; one without is an attempt left in
 *  flight past its lease after its run ended. */
export function writeKind(write: FailedWriteView): WriteKind {
	return write.failure_code === undefined ? 'stranded' : 'failed';
}

export const WRITE_FILTERS: readonly { id: WriteKind | 'all'; label: string }[] = [
	{ id: 'all', label: 'All' },
	{ id: 'stranded', label: 'Stranded' },
	{ id: 'failed', label: 'Failed' }
];

// --- import drain ---------------------------------------------------------------

export type DrainVerdict = 'draining' | 'flat' | 'short' | 'unmeasurable';

/** Where one organisation's series stands against the gate: falling, not
 *  falling, too short to say, or unreadable at one end. */
export function drainVerdict(gate: GateWindow): DrainVerdict {
	if (gate.gap !== null) {
		return gate.gap === 'short' ? 'short' : 'unmeasurable';
	}
	return (gate.fall ?? 0) > 0 ? 'draining' : 'flat';
}

export const DRAIN_WORD: Record<DrainVerdict, { label: string; tone: Tone }> = {
	draining: { label: 'draining', tone: 'ok' },
	flat: { label: 'not falling', tone: 'bad' },
	short: { label: 'too few runs', tone: 'soon' },
	unmeasurable: { label: 'unmeasurable', tone: 'warn' }
};

// --- impersonations --------------------------------------------------------------

export function impersonationKey(row: ImpersonationView): string {
	return `${row.event}-${row.at}-${row.actor}-${row.target}`;
}

/** The start events no later stop answers, by `impersonationKey`: the
 *  impersonations that may still be live. Read oldest first per actor and
 *  target, so a stop closes the start before it and nothing after. */
export function openImpersonations(events: readonly ImpersonationView[]): Set<string> {
	const open = new Map<string, string>();
	const ordered = [...events].sort((a, b) => a.at - b.at);
	for (const row of ordered) {
		const pair = `${row.actor}\u0000${row.target}`;
		if (row.event === 'user_impersonated') {
			open.set(pair, impersonationKey(row));
		} else {
			open.delete(pair);
		}
	}
	return new Set(open.values());
}

// --- guide images -------------------------------------------------------------------

export interface GuideImage {
	src: string;
	alt: string;
	/** Uploaded here, rather than loaded from another site. */
	own: boolean;
}

const IMAGE = /!\[([^\]]*)\]\(\s*<?([^\s)>]+)>?(?:\s+"[^"]*")?\s*\)/g;

/** Every picture the body's Markdown shows, once each, in the order they
 *  first appear. */
export function guideImages(body: string): GuideImage[] {
	const found = new Map<string, GuideImage>();
	for (const match of body.matchAll(IMAGE)) {
		const src = match[2];
		if (!found.has(src)) {
			found.set(src, { src, alt: match[1], own: src.startsWith('/v1/guides/images/') });
		}
	}
	return [...found.values()];
}
