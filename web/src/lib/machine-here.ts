// What the console says about the machine it is being read on: which one it
// is, whether the seller is in the Teachouse app or in a browser, and what to
// do about a machine that was signed out from the console. Pure, so it tests
// without a component.
//
// The founder's 0.7.0 review asked for the first of these in as many words:
// every surface must say which machine the seller is at and whether they are
// in the app. The console is served from the same build to both hosts, so
// nothing on the screen distinguished them, and a seller reading "Signed out"
// beside a machine could not tell whether it was the one in front of them.

import { ApiFailure } from '$lib/api';
import { dayMonth } from '$lib/entitlement';

/** Which machine this copy of the console is being read on.
 *
 * `device` is null in a browser, which is no machine at all, and null in an
 * application too old to send its own identity — those are different facts and
 * `inApp` is what tells them apart. */
export interface MachineHere {
	inApp: boolean;
	device: { id: string; name: string } | null;
}

/** Where the seller is, in one sentence.
 *
 * Short enough for a control's title and complete enough for a line on the
 * Account page, because it is both: the top strip's account link, the phone's
 * account avatar and the Preferences head all say the same thing, and a
 * seller who reads two different sentences in two places has to work out
 * whether they mean the same.
 *
 * The app-with-no-name arm is not a defect to hide. A machine whose check-in
 * has not landed yet, and an application older than the two fields, both reach
 * it; naming no machine is true in both, and inventing one is not. */
export function whereYouAre(machine: MachineHere): string {
	if (!machine.inApp) {
		return 'You are in a browser, not the Teachouse app.';
	}
	return machine.device === null
		? 'You are in the Teachouse app on this machine.'
		: `You are on ${machine.device.name} in the Teachouse app.`;
}

/** The control that brings a signed-out machine back, named the same wherever
 *  it is offered: the banner over every page and the row for this machine on
 *  the Machines list. */
export const SIGN_BACK_IN = 'Sign this machine back in';

/** What the banner says to a seller standing at a machine that was signed out
 *  from the console.
 *
 * The date rather than an age, because this is the founder's own case: he
 * signed both machines out from his phone's browser in the morning and met the
 * consequence for days afterwards, by which time "5 days ago" names nothing he
 * remembers doing and a date does.
 *
 * The date is omitted rather than guessed where the registry read has not
 * landed, because the sentence is about an act the seller performed and a
 * wrong date reads as a different act. */
export function signedOutHere(at: number | null): string {
	const when = at === null ? '' : ` on ${dayMonth(at)}`;
	return (
		`This machine was signed out from the console${when}. Sign it back in to hold ` +
		'marketplace logins here.'
	);
}

/** What the seller is told when signing the machine back in did not work.
 *
 * The server's own sentence wherever it sent one, because the refusal that
 * matters here is the machine cap: the restore route counts the machines
 * already standing and refuses at `devices_max`, and "try again" would be
 * advice to repeat something that will refuse identically for ever. The
 * server's sentence names the cap and the plan; ours names neither.
 *
 * Our own words wherever the call carried none — a network that dropped, a
 * body that did not parse, a bare status — which is the one case where trying
 * again is the right advice. Read off the structured body rather than off
 * `ApiFailure.message`, because that falls back to "request failed with 500",
 * which is a sentence for us and not for a seller. */
export function signBackInRefusal(failure: unknown): string {
	const said =
		failure instanceof ApiFailure ? (failure.body?.errors?.[0]?.message ?? '').trim() : '';
	return said.length > 0 ? said : NOT_SIGNED_BACK_IN;
}

/** What stands where the server said nothing readable. */
export const NOT_SIGNED_BACK_IN =
	'This machine was not signed back in. Check your connection and try again.';
