// The operator console's own model: whether the signed-in human is an
// operator at all, what a refusal on that surface means, how a signup series
// becomes a bar list, and whether the app is currently being driven under
// somebody else's identity. Pure, so it tests without a component.

import { ApiFailure, type AdminUserView, type DayCount, type SignupsView } from '$lib/api';
import type { IdentityUser } from '$lib/auth-client';

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

// ------------------------------- the two planes' user lists, joined

/**
 * One row of the Identity users page: an identity account, and the platform
 * user that account provisioned if there is one.
 *
 * The identity account is the spine because it is the list that can be
 * searched and bounded — the platform read answers every app user at once.
 * `platform` being null is an ordinary state, not a fault: an identity account
 * that never reached the session exchange has no `app_user` row at all.
 */
export interface AdminUserRow {
	identity: IdentityUser;
	platform: AdminUserView | null;
}

export interface MergedUsers {
	rows: AdminUserRow[];
	/**
	 * App users this listing cannot show: ones carrying no `auth_subject`, and
	 * ones whose subject is not on the identity page in front of the operator.
	 *
	 * Counted rather than dropped silently, because `auth_subject` is nullable
	 * and a provisioned user with no identity subject is exactly the row that
	 * would otherwise vanish between the two planes with nothing saying so.
	 */
	unlinked: number;
}

/**
 * Join the identity plane's accounts to the platform's own user rows.
 *
 * The key is `app_user.auth_subject`, which the operator surface omits where
 * there is none: a row carrying no subject can never name an identity
 * account, so it is never matched — not even to an identity account whose own
 * id happens to be missing. The guard is "is this a string" rather than a
 * comparison against one absent value, so neither an omitted key nor an
 * explicit null can become a map key. A subject claimed by two app users
 * would be a broken unique index rather than a case to resolve, so the first
 * row wins and the second is counted as unlinked.
 */
export function mergeUsers(
	identity: readonly IdentityUser[],
	platform: readonly AdminUserView[]
): MergedUsers {
	const bySubject = new Map<string, AdminUserView>();
	let unlinked = 0;
	for (const user of platform) {
		const subject = user.auth_subject;
		if (typeof subject !== 'string' || bySubject.has(subject)) {
			unlinked += 1;
			continue;
		}
		bySubject.set(subject, user);
	}
	const rows = identity.map((account) => ({
		identity: account,
		platform: bySubject.get(account.id) ?? null
	}));
	// Every app user whose subject no listed account claims: the listing is
	// search-narrowed, so this is usually "the rest of the platform" rather
	// than anything wrong.
	const matched = rows.filter((row) => row.platform !== null).length;
	return { rows, unlinked: unlinked + (bySubject.size - matched) };
}

/**
 * The sign-ins column, in words.
 *
 * `null` is "not read yet" rather than zero: the identity service lists
 * sessions one account at a time, so a fifty-row page would be fifty requests
 * and the count arrives only for the account an operator opened. Zero is its
 * own answer — an account with no live session cannot be signed out of
 * anything.
 */
export function sessionWords(count: number | null): string {
	if (count === null) {
		return 'not read';
	}
	if (count === 0) {
		return 'none';
	}
	return count === 1 ? '1 sign-in' : `${count} sign-ins`;
}

/**
 * Whether the identity trail is visible from the database the API reads.
 *
 * `last_sign_in_at` is absent both for an account that has never signed in
 * and for a deployment whose `auth` schema this database cannot see, and no
 * single row can tell those apart. A page of rows can: if not one of them
 * carries a sign-in, the trail is the likelier explanation, and the page says
 * so once instead of printing "never" against every account.
 */
export function signInTrailVisible(users: readonly AdminUserView[]): boolean {
	return users.some((user) => user.last_sign_in_at !== undefined);
}
