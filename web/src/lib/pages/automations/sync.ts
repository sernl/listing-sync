// The Marketplace Sync page's rows: one card per engine run, the same set as
// activity-log lines, the per-marketplace count the left column carries, and
// the words the "Open questions" header control uses. Pure, so it tests
// without a component.

import type { JobHead } from '$lib/api';
import type { LogEntry } from '$lib/ActivityLog.svelte';
import { agoLabel } from '$lib/elapsed';
import { platformTitle, SHORT_NAME } from '$lib/platforms';

export interface RunRow {
	job: string;
	href: string;
	/** Where the run was sent, which is what tells one run from another at a
	 *  glance; the identifier follows it in the meta line. */
	title: string;
	meta: string;
	/** The instant the run was created, so the template can render the absolute
	 *  time in the seller's own locale without this module carrying a
	 *  locale-dependent string. */
	at: number;
}

export function runRows(jobs: readonly JobHead[], now: number): RunRow[] {
	return jobs.map((job) => ({
		job: job.job,
		href: `/sync/${job.job}`,
		title: platformTitle(job.inventory),
		meta: `Run ${job.job.slice(0, 8)}… · started ${agoLabel(job.created_at, now)}`,
		at: job.created_at
	}));
}

export function runLog(jobs: readonly JobHead[], now: number): LogEntry[] {
	return jobs.map((job) => ({
		id: job.job,
		what: `${SHORT_NAME[job.inventory]} · run ${job.job.slice(0, 8)}…`,
		at: agoLabel(job.created_at, now)
	}));
}

/** The header control's own words. A figure this page could not read is not a
 *  figure of zero, so the control drops the count rather than claiming one. */
export function openQuestionsLabel(open: number | null): string {
	return open === null ? 'Open questions' : `Open questions (${open})`;
}

/** Why the schedule card's controls are disabled.
 *
 * The cadence lives on the seller's own computer, and no route on this server
 * reads or writes one, so a Save that appeared to set a schedule would be a
 * promise nothing keeps. Every disabled control in this console states its
 * reason, because one that does not reads as a fault. */
export const SCHEDULE_NOT_SETTABLE = 'Teachouse cannot set a schedule yet.';

/** What the settings card is, stated on the card rather than only in the
 *  banner: no route stores a cadence, so a control's position here is not the
 *  current value of anything. Both Automations settings cards say this, in the
 *  same words, because they are the same kind of claim. */
export const SETTINGS_ARE_A_PREVIEW =
	'What a schedule would offer here once Teachouse can set one. None of these is switched ' +
	'on, and nothing here is saved.';

export const NO_RUN_YET =
	'Start one from Resources: choose what to send, and we take it from there.';

export const SCHEDULE_TITLE = 'The schedule runs on your own computer';

export const SCHEDULE_BODY =
	'TES and TPT are only ever reached from your own computer, signed in as you, so the timer ' +
	'runs there too. Setting it from here is not built yet.';

/** How often the console would ask a device to look for changes. Laid out so
 *  the shape can be corrected before anything is written against it. */
export const CADENCE_OPTIONS = ['Every check-in', 'Once a day', 'Off'] as const;
export type Cadence = (typeof CADENCE_OPTIONS)[number];
