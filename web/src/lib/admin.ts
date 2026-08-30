// The operator console's own model: whether the signed-in human is an
// operator at all, what a refusal on that surface means, how a signup series
// becomes a bar list, and whether the app is currently being driven under
// somebody else's identity. Pure, so it tests without a component.

import { ApiFailure, type DayCount, type SignupsView } from '$lib/api';

/** What the operator probe has established so far.
 *
 * `checking` is the state before any answer: the Admin group is hidden then,
 * because a group that appeared and vanished would read as a bug rather than
 * as a permission. */
export type OperatorVerdict = 'checking' | 'operator' | 'outsider';

export interface ProbeReading {
	/** Whether the probe came back with a body. */
	answered: boolean;
	/** The failure it ended in, or null while it has not ended in one. */
	failure: unknown;
}

/**
 * The verdict a probe reading carries.
 *
 * Any failure at all reads as `outsider`, deliberately. The refusal a
 * non-operator gets is the blank 401 an anonymous caller gets, so this client
 * cannot tell "not an operator" from "session expired" and must not pretend
 * to; and a surface that appeared on a network blip would be worse than one
 * that stays hidden until a read succeeds.
 */
export function operatorVerdict(reading: ProbeReading): OperatorVerdict {
	if (reading.answered) {
		return 'operator';
	}
	return reading.failure === null || reading.failure === undefined ? 'checking' : 'outsider';
}

/** Why an operator page has nothing to show. */
export type OutsiderReason = 'not-an-operator' | 'unconfigured' | 'unreadable';

/**
 * What a refusal on the operator surface actually says.
 *
 * The 401 is the deliberate blank refusal, and it is the answer both a seller
 * and an anonymous caller get. `backoffice_unavailable` is the one condition
 * the server names outright: the deployment was started without a backoffice
 * database, so it serves no operator surface rather than half of one, and
 * that is a deployment fact rather than a permission one.
 */
export function outsiderReason(failure: unknown): OutsiderReason {
	if (failure instanceof ApiFailure) {
		if (failure.code() === 'backoffice_unavailable') {
			return 'unconfigured';
		}
		if (failure.status === 401) {
			return 'not-an-operator';
		}
	}
	return 'unreadable';
}

/** One day of the signup chart: a UTC day start, and what each plane counted
 *  on it. `identity` is null where the identity trail is not visible from
 *  this deployment, which is not the same as a day on which nobody signed
 *  up. */
export interface SignupBar {
	day: number;
	provisioned: number;
	identity: number | null;
}

/** How many days the overview charts. Bounded here rather than by the server,
 *  which serves the whole series: this is a rendering decision. */
export const SIGNUP_DAYS = 14;

function indexByDay(days: readonly DayCount[]): Map<number, number> {
	return new Map(days.map((entry) => [entry.day, entry.count]));
}

/**
 * The newest `limit` days across both series, newest first.
 *
 * The union of the two sets of days rather than either one alone: a day on
 * which somebody reached the identity service and never came back carries an
 * identity count and no provisioned count, and dropping it would hide exactly
 * the gap between the two planes the chart exists to show. A day present in
 * one series and missing from the other counts zero there, because the series
 * is a complete daily aggregate and an absent day genuinely had none.
 */
export function signupSeries(view: SignupsView, limit: number = SIGNUP_DAYS): SignupBar[] {
	const identity = view.identity === undefined ? null : indexByDay(view.identity);
	const provisioned = indexByDay(view.provisioned);
	const days = new Set<number>([...provisioned.keys(), ...(identity?.keys() ?? [])]);
	return [...days]
		.sort((left, right) => right - left)
		.slice(0, Math.max(0, limit))
		.map((day) => ({
			day,
			provisioned: provisioned.get(day) ?? 0,
			identity: identity === null ? null : (identity.get(day) ?? 0)
		}));
}

/** The tallest count in the rendered window, which every bar is drawn against.
 *  Zero for an empty window, and the bar width guards that case. */
export function signupPeak(rows: readonly SignupBar[]): number {
	let peak = 0;
	for (const row of rows) {
		peak = Math.max(peak, row.provisioned, row.identity ?? 0);
	}
	return peak;
}

/**
 * A bar's width as a percentage of the panel.
 *
 * A non-zero count never renders as nothing: it floors at a visible sliver, so
 * "one signup" and "no signups" cannot look alike on a day beside a tall one.
 */
export function barWidth(count: number, peak: number): number {
	if (count <= 0 || peak <= 0) {
		return 0;
	}
	return Math.max(2, Math.round((count / peak) * 100));
}

/** The day label the chart puts beside each row: the UTC calendar date, which
 *  is the day the server aggregated by. Rendering it in local time would
 *  relabel the boundary the count was taken at. */
export function dayLabel(day: number): string {
	return new Date(day).toISOString().slice(0, 10);
}

/** The identity-plane session, reduced to what the banner needs. */
export interface ImpersonatedSession {
	user: { name?: string | null; email?: string | null };
	session: { impersonatedBy?: string | null };
}

export interface ImpersonationState {
	/** Who the console is currently acting as, in the words the banner shows. */
	who: string;
}

/**
 * Whether this console is being driven under somebody else's identity, and
 * whose.
 *
 * The signal is `session.impersonatedBy`, which better-auth's admin plugin
 * fills with the acting admin's id and leaves absent otherwise. It is read
 * rather than remembered: a page reloaded mid-impersonation must raise the
 * banner from the session it finds, not from state the reload destroyed.
 *
 * The name is preferred over the address and the address stands in for a blank
 * name, because a banner reading "Signed in as" followed by nothing would be
 * the one case where the warning fails to warn.
 */
export function impersonationState(session: ImpersonatedSession | null): ImpersonationState | null {
	const actor = session?.session.impersonatedBy;
	if (session === null || actor === null || actor === undefined || actor === '') {
		return null;
	}
	const name = session.user.name?.trim();
	if (name !== undefined && name.length > 0) {
		return { who: name };
	}
	const email = session.user.email?.trim();
	return { who: email !== undefined && email.length > 0 ? email : 'another account' };
}

/** Why the identity-plane user list has nothing to show. */
export type IdentityAdminRefusal = 'not-identity-admin' | 'unreadable';

/**
 * What a refusal from better-auth's admin plugin means.
 *
 * The plugin answers 403 to a signed-in human whose identity account does not
 * carry an admin role, which is a different fact from the platform operator
 * marking: the two roles are granted separately and by hand, and an operator
 * reading this page is being told which of the two they are missing.
 */
export function identityAdminRefusal(status: number | undefined): IdentityAdminRefusal {
	return status === 401 || status === 403 ? 'not-identity-admin' : 'unreadable';
}
