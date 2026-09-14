// The one place the console speaks to the desktop application it may be
// running inside. Everything else takes an invoker as an argument, so the rest
// of the console tests without a webview and without a Tauri stub.
//
// The reader below looks for the global Tauri exposes under
// `app.withGlobalTauri`, rather than importing `@tauri-apps/api`, so the
// console bundle acquires no dependency for a path only one of its two hosts
// can take.
//
// A null invoker means a console served outside the application: a browser, or
// the dev server opened directly rather than through the app window. It never
// means a seller standing in the application while nothing runs, because that
// flag and this console ship in the same desktop build — an application able
// to render this page is an application able to invoke.

import type { Marketplace } from '$lib/generated/vocab';

/** How a command is called. The whole surface the console needs, named as a
 *  type so every consumer can be handed a fake. */
export type Invoke = (command: string, args: Record<string, unknown>) => Promise<unknown>;

interface TauriGlobal {
	core?: { invoke?: unknown };
}

/** The application's own invoker, or null when this console is a web page.
 *
 * Null is the ordinary answer in a browser and is not a fault. A caller reads
 * it as "this computer cannot run the work from here", never as an error to
 * report. */
export function desktopInvoker(): Invoke | null {
	if (typeof window === 'undefined') {
		return null;
	}
	const tauri = (window as unknown as { __TAURI__?: TauriGlobal }).__TAURI__;
	const invoke = tauri?.core?.invoke;
	return typeof invoke === 'function' ? (invoke as Invoke) : null;
}

/** The desktop command that reads one shop and posts the list of what is in
 *  it. Answers once the enumeration has landed, so a refusal — no session,
 *  no entitlement, a shop that would not open — is this command's own error
 *  rather than a run that sits in `reading` for ever.
 *
 * Named here rather than written at the call site so the string the console
 * sends and the string the application registers are compared in one place. */
export const START_IMPORT = 'start_import';

/** The desktop command that reads the resources the seller ticked.
 *
 * Answers as soon as the describe pass is running, not when it finishes: the
 * pass posts pages for as long as it takes and the ledger is what says how
 * far it has got. A console that waited for this to resolve would show
 * nothing for the length of a shop. */
export const CONTINUE_IMPORT = 'continue_import';

/** The desktop command that opens one marketplace's own login page on this
 *  computer and files the session in the platform keychain.
 *
 * The whole of the D1 connect path, and the reason there is no server-side
 * equivalent to fall back to: for a marketplace with no official API the login
 * happens on the seller's machine or it does not happen. */
export const CONNECT_MARKETPLACE = 'connect_marketplace';

/** The desktop command that removes one marketplace's session from this
 *  computer's keychain. The only way a captured jar leaves a device. */
export const FORGET_SESSION = 'forget_session';

/** The desktop command that puts this machine in the seller's registry.
 *
 * Named here for the reason `START_IMPORT` is: the string the console sends and
 * the string the application registers are compared in one place. */
export const DEVICE_CHECK_IN = 'device_check_in';

/** What one check-in answered.
 *
 * `reached` is whether the registry may now have gained a row, which is the
 * caller's cue to read it again. False in every browser, where there is no
 * application to register and nothing failed. False also when the application
 * could not reach the server: the command reports that as an ordinary answer
 * rather than a rejection, and a list refetched over the same dead connection
 * would only fail twice.
 *
 * `detail` is the application's own sentence for why the check-in did not
 * reach us, and is null on every other path: a success, a browser, and an
 * application too old to carry the field.
 *
 * `revoked` is the machine's own copy of what the heartbeat was told: the
 * seller signed this machine out from the console, and it has wiped its
 * marketplace logins. It is what lets the console say so on the machine it
 * happened to, rather than only in a list of every machine.
 *
 * `device` is which machine this is, by the identifier and the name the
 * registry holds. Null in a browser, which is no machine at all, and null from
 * an application too old to send the two fields — an absent name has to read
 * as "not known" rather than as a machine called nothing. */
export interface CheckInHere {
	reached: boolean;
	detail: string | null;
	revoked: boolean;
	device: { id: string; name: string } | null;
}

/** What a check-in that never happened answers: a browser, and a rejection.
 *
 * `revoked: false` is not a claim that the machine stands — nothing was asked
 * — and no caller reads it as one, because every one of them tests `reached`
 * first. */
const NOTHING_CHECKED_IN: CheckInHere = {
	reached: false,
	detail: null,
	revoked: false,
	device: null
};

/** Ask this machine to register itself and check in, and read what it said.
 *
 * The application registers itself on its own coordinator as well, and this is
 * the half that makes it prompt. That coordinator's first pass runs at
 * start-up, which is before the seller has signed in and therefore has no
 * session to register under; after it, the check-in runs every five minutes on
 * a computer and on each resume on a phone — so without this call a seller who
 * has just signed in waits minutes in front of an empty machine list with no
 * way to tell a slow registration from a broken one. It is also the one call
 * that learns, under a session, that this machine was signed out from the
 * console.
 *
 * A rejection is `reached: false` with no detail. The three causes — an
 * application too old to know the command, a page it takes no commands from,
 * and a genuine refusal — mean the same thing here, which is that no row was
 * added, and none of them is worded for a seller. */
export async function checkInHere(invoke: Invoke | null): Promise<CheckInHere> {
	if (invoke === null) {
		return NOTHING_CHECKED_IN;
	}
	try {
		return deviceState(await invoke(DEVICE_CHECK_IN, {}));
	} catch {
		return NOTHING_CHECKED_IN;
	}
}

/** The application's `DeviceState`, read field by field.
 *
 * One reader rather than one per field, because every field has the same
 * boundary problem and the same answer to it: the console is served from the
 * control plane and updates when we deploy, while the application updates
 * when the seller takes an update, so this console is routinely the newer of
 * the two and every field here may simply be absent. Absent reads as "not
 * said" in each case, never as a value.
 *
 * `revoked` is believed only beside a check-in that reached us: an application
 * that could not reach the control plane learnt nothing about whether the
 * seller signed this machine out, and a banner raised over a dropped
 * connection would accuse them of an act they did not perform.
 *
 * The device is both fields or neither. A name with no identifier cannot be
 * acted on — the restore route addresses a device by id — and an identifier
 * with no name cannot be shown.
 *
 * `detail` is `ControlPlaneError`'s own sentence: four of them, each already
 * worded for a person, naming no credential, no jar and no host but our own
 * control plane. */
function deviceState(answer: unknown): CheckInHere {
	if (typeof answer !== 'object' || answer === null) {
		return NOTHING_CHECKED_IN;
	}
	const reached = 'reached_server' in answer && answer.reached_server === true;
	const revoked = reached && 'revoked' in answer && answer.revoked === true;
	const detail = 'detail' in answer ? answer.detail : undefined;
	const id = 'device_id' in answer ? answer.device_id : undefined;
	const name = 'device_name' in answer ? answer.device_name : undefined;
	const named =
		typeof id === 'string' && id.length > 0 && typeof name === 'string' && name.length > 0;
	return {
		reached,
		detail: typeof detail === 'string' && detail.length > 0 ? detail : null,
		revoked,
		device: named ? { id, name } : null
	};
}

/** What asking this computer to run an import produced.
 *
 * `refused` ordinarily carries the application's own sentence, unaltered. Its
 * three causes — no marketplace session held for the request's source, a closed
 * entitlement, and a pass already running for this request — are the
 * application's to word, and `CommandError` is a newtype over a string, so
 * they arrive as prose rather than as codes the console could branch on. It is
 * rendered rather than interpreted.
 *
 * The one substitution is `ORIGIN_NOT_GRANTED`, which is a refusal the
 * application words for a developer and not for a seller. It is carried as a
 * refusal rather than as a fourth arm because both pages read the arms they
 * know and fall through the rest to the success path, so a new arm would render
 * an unstarted import as a started one. */
export type StartOutcome =
	| { kind: 'started' }
	| { kind: 'refused'; detail: string }
	| { kind: 'unsupported' }
	| { kind: 'unavailable' };

/** What a seller is told when the application they are standing in has no
 *  start_import to call. Their remedy is an update, which is a different act
 *  from any of the three refusals and must not read as one.
 *
 *  Reachable by a seller, not only by a developer running the dev server. A
 *  shipped bundle carries the console and the command table as one artifact,
 *  so those two cannot disagree — but the console is also served from the
 *  control plane's own origin, and that copy updates when we deploy while the
 *  application on the seller's machine updates when they take an update. The
 *  console can therefore be the newer of the two, which is exactly this
 *  arm. */
export const APP_TOO_OLD =
	'This version of the Teachouse app cannot start an import. Update it and try again.';

/** What a seller is told when the application refuses because the page asking
 *  is not one it accepts commands from.
 *
 * Substituted for the application's own words, which is the only place this
 * file overwrites them. Those words name the window, the webview, every origin
 * the capability grants and the permission behind each grant, so rendering them
 * would publish the fence's shape to anyone who can reach the page.
 *
 * Not a seller's fault and not a seller's remedy, which is why it reads as
 * neither: either the console is served from an origin
 * `apps/desktop/src-tauri/capabilities/default.json` does not name, which is
 * ours to fix and not theirs, or the window is holding a development server. */
export const ORIGIN_NOT_GRANTED = 'This page is not one the Teachouse app accepts commands from.';

/** Ask this computer to read the shop one run names.
 *
 * `unavailable` where there is no invoker, which is every browser: the caller
 * shows the seller how to run it on the machine that holds the session rather
 * than reporting a failure, because nothing failed.
 *
 * The run rather than a request: an import is an import, and the run is the
 * row the console, the server and this computer all address it by.
 *
 * `takeover` is the seller having said, in so many words, that this machine
 * should take a run another one holds. It defaults to false, and that default
 * is the safe half: an ordinary press must not wrench a run out of a phone
 * that is reading a shop right now, so the application refuses and the caller
 * offers the confirmation instead. */
export async function startImportHere(
	invoke: Invoke | null,
	run: string,
	takeover = false
): Promise<StartOutcome> {
	return ranOnRun(invoke, START_IMPORT, run, takeover);
}

/** Ask this computer to read the resources the seller ticked.
 *
 * The second half of one import, and a separate command because the seller
 * stands between them: the first posts what is in the shop, the seller says
 * which of it to bring, and this one reads those.
 *
 * `takeover` means what it means above, and is needed here for the same
 * reason: a seller who ticked on a laptop and continues on a phone is moving
 * the run, which is a thing to confirm rather than to do by accident. */
export async function continueImportHere(
	invoke: Invoke | null,
	run: string,
	takeover = false
): Promise<StartOutcome> {
	return ranOnRun(invoke, CONTINUE_IMPORT, run, takeover);
}

/** The desktop command that puts one run down on this computer.
 *
 * Its own name rather than a flag on the two above, because it is a different
 * act: starting asks this machine to take a run, and stopping asks it to stop
 * reading one — which it can do while offline, holding work the server has not
 * acknowledged yet. */
export const STOP_IMPORT = 'stop_import';

/** What asking this computer to stop one run produced.
 *
 * `stopped` carries whether the server has acknowledged it. A machine that is
 * offline stops reading immediately and cannot say the run is stopped
 * anywhere else yet, and the seller is owed that difference: the work on this
 * machine has ended either way, and the run's own page will agree once the
 * device reaches us. A caller renders `serverPending` as "Stopped on this
 * device; server confirmation pending" rather than as a failure, because
 * nothing failed. */
export type StopOutcome =
	| { kind: 'stopped'; serverPending: boolean; recorded: boolean }
	| { kind: 'refused'; detail: string }
	| { kind: 'unsupported' }
	| { kind: 'unavailable' };

/** Ask this computer to stop reading the shop one run names.
 *
 * The console's Stop also settles the run on the server, and that is the
 * authority; this is the half only the device can do, which is ending the
 * marketplace work already in flight. Both are needed: without the server the
 * run would be reclaimed, and without this the device would go on reading a
 * run nobody is waiting for. */
export async function stopImportHere(invoke: Invoke | null, run: string): Promise<StopOutcome> {
	if (invoke === null) {
		return { kind: 'unavailable' };
	}
	try {
		const answer = await invoke(STOP_IMPORT, { run });
		// Narrowed rather than asserted, and read conservatively: an answer
		// whose shape this console does not recognise is not a reason to tell
		// the seller the stop failed, and the honest reading of a field that
		// is not there is that the server has yet to confirm.
		const confirmed =
			typeof answer === 'object' &&
			answer !== null &&
			'server_pending' in answer &&
			answer.server_pending === false;
		// Whether the machine wrote the stop down, which is a different fact
		// from waiting on the server: an unrecorded stop can be undone by a
		// restart, and the seller is owed that distinction rather than one
		// sentence covering both.
		const recorded =
			typeof answer === 'object' &&
			answer !== null &&
			'recorded' in answer &&
			answer.recorded === true;
		return { kind: 'stopped', serverPending: !confirmed, recorded };
	} catch (caught) {
		const detail = refusalText(caught, STOP_REFUSED_SILENTLY);
		if (originNotGranted(detail, STOP_IMPORT)) {
			return { kind: 'refused', detail: ORIGIN_NOT_GRANTED };
		}
		if (signedOut(detail)) {
			return { kind: 'refused', detail: DEVICE_SIGNED_OUT };
		}
		return unknownCommand(detail) ? { kind: 'unsupported' } : { kind: 'refused', detail };
	}
}

/** The desktop command that reads whether THIS machine holds a session for one
 *  marketplace.
 *
 * A different question from the organisation's connection standing, which is
 * the server's and says that some device is connected. An import runs on the
 * machine the seller is standing at, so "connected" on the console and "can
 * read your shop from here" are two facts, and conflating them is what let a
 * seller press Start on a phone that held no session. */
export const SESSION_STATUS = 'session_status';

/** What this machine knows about its own session for one marketplace.
 *
 * `known` is an answer and carries it; `unavailable` is a browser, where the
 * question cannot be asked at all and no answer may be inferred. The two are
 * kept apart deliberately: an unanswered question rendered as "not connected"
 * is the same defect one layer up, and a caller showing "Connect this device"
 * to a seller in a browser would be offering a remedy that does not exist
 * there. */
export type LocalSessionOutcome =
	| { kind: 'known'; connected: boolean }
	| { kind: 'refused'; detail: string }
	| { kind: 'unavailable' };

/** Ask this computer whether it holds a session for one marketplace.
 *
 * Read-only: it opens nothing, captures nothing and changes nothing, so a
 * caller may ask it on entry, on visibility and before a start without
 * costing the seller anything. */
export async function sessionStatusHere(
	invoke: Invoke | null,
	marketplace: Marketplace
): Promise<LocalSessionOutcome> {
	if (invoke === null) {
		return { kind: 'unavailable' };
	}
	try {
		const answer = await invoke(SESSION_STATUS, { marketplace });
		// An answer whose shape this console does not recognise is unavailable
		// rather than disconnected: absent and unknown are different facts,
		// and only one of them has a remedy the seller can act on.
		if (typeof answer === 'object' && answer !== null && 'connected' in answer) {
			const connected = answer.connected;
			if (typeof connected === 'boolean') {
				return { kind: 'known', connected };
			}
		}
		return { kind: 'unavailable' };
	} catch (caught) {
		const detail = refusalText(caught, SESSION_UNREADABLE);
		if (originNotGranted(detail, SESSION_STATUS)) {
			return { kind: 'refused', detail: ORIGIN_NOT_GRANTED };
		}
		// An application too old to answer is not a machine with no session:
		// it is a question that could not be asked, which is what
		// `unavailable` means everywhere else in this module.
		return unknownCommand(detail) ? { kind: 'unavailable' } : { kind: 'refused', detail };
	}
}

/** Both halves' shared body. One classification of a rejection, so a start and
 * a continue cannot come to read this computer's refusals differently.
 *
 * `takeover` is sent on every call rather than only when true, because Tauri
 * matches an argument by name and a command whose parameter is absent reads
 * its own default: sending it always means the console and the application
 * agree on one answer rather than two defaults that could drift. */
async function ranOnRun(
	invoke: Invoke | null,
	command: string,
	run: string,
	takeover: boolean
): Promise<StartOutcome> {
	if (invoke === null) {
		return { kind: 'unavailable' };
	}
	try {
		await invoke(command, { run, takeover });
		return { kind: 'started' };
	} catch (caught) {
		const detail = refusalText(caught, IMPORT_REFUSED_SILENTLY);
		if (originNotGranted(detail, command)) {
			return { kind: 'refused', detail: ORIGIN_NOT_GRANTED };
		}
		if (signedOut(detail)) {
			return { kind: 'refused', detail: DEVICE_SIGNED_OUT };
		}
		return unknownCommand(detail) ? { kind: 'unsupported' } : { kind: 'refused', detail };
	}
}

/** What asking this computer to connect or forget one marketplace produced.
 *
 * The same four answers `StartOutcome` carries, with `done` where that one has
 * `started`, plus two the connect alone can give.
 *
 * `opening` is the sign-in replacing this very page, which is what a connect
 * does where the application has one window: there is no second window for the
 * marketplace to open in, so the console goes away and the answer cannot come
 * back through the promise that asked for it. It arrives instead in the address
 * the application returns to, read by `connectReturn` in
 * `$lib/pages/marketplaces/view`. A caller seeing `opening` says nothing and
 * leaves the card busy, because the page it would say it on is about to unload.
 *
 * `signedOut` is the machine having been signed out from the console. The
 * application checks in before it opens anything, so the sign-in never starts:
 * a machine the server calls revoked wipes every marketplace session it is
 * handed, and before this arm existed the seller typed a marketplace password
 * into a window whose capture was then thrown away. Refusing in front of the
 * password is the whole point of it.
 *
 * `unavailable` is the ordinary browser answer and is not a failure. The
 * caller shows the seller where the act can be performed instead. */
export type SessionOutcome =
	| { kind: 'done' }
	| { kind: 'opening' }
	| { kind: 'signedOut' }
	| { kind: 'refused'; detail: string }
	| { kind: 'unsupported' }
	| { kind: 'unavailable' };

/** What a seller is told when the application they are standing in has no
 *  connect_marketplace to call. Its own sentence rather than `APP_TOO_OLD`,
 *  which names an import: a seller pressing Connect and reading about imports
 *  reads it as the wrong button rather than as an old application. */
export const APP_CANNOT_CONNECT =
	'This version of the Teachouse app cannot connect a marketplace. Update it and try again.';

/** What a seller is told when the application cannot forget a session.
 *
 * Says what still stands, because a disconnect that half-happened is the one
 * case where saying nothing leaves a login on a machine the seller believes is
 * clear. The server half runs regardless, so the sentence is about the machine
 * alone. */
export const APP_CANNOT_FORGET =
	'This version of the Teachouse app cannot remove a marketplace login. The login is still on ' +
	'this machine; update the app, or sign the machine out below.';

/** Ask this computer to open one marketplace's login and keep the session.
 *
 * The window the application opens is the marketplace's own page, and the
 * promise settles only when the sign-in completes, the seller closes the
 * window, or the application's deadline passes — so a caller shows a busy
 * state for as long as this is pending.
 *
 * Nothing about the marketplace is composed here and no request to it
 * originates here: this hands a marketplace name to the application running on
 * the seller's own machine, which is the only place D1 permits the login to
 * happen. */
export async function connectHere(
	invoke: Invoke | null,
	marketplace: Marketplace
): Promise<SessionOutcome> {
	if (invoke === null) {
		return { kind: 'unavailable' };
	}
	try {
		return connectOutcome(await invoke(CONNECT_MARKETPLACE, { marketplace }));
	} catch (caught) {
		return refusal(caught, CONNECT_MARKETPLACE, CONNECT_REFUSED_SILENTLY);
	}
}

/** What the application said a connect did, out of its `ConnectOutcome`.
 *
 * `done` on anything else, and that default is what keeps an older application
 * working: before `ConnectOutcome` existed the command answered a bare session
 * status, which carries no `outcome` at all and meant a completed capture. So
 * an unrecognised answer reads as the success it used to be rather than as a
 * state this console would then wait forever in.
 *
 * The two named arms are both "nothing was captured and the page must not
 * claim otherwise": `opening` because the answer is coming back through the
 * address instead, `signed_out` because the application refused to open the
 * sign-in at all. */
function connectOutcome(answer: unknown): SessionOutcome {
	const said =
		typeof answer === 'object' && answer !== null && 'outcome' in answer
			? answer.outcome
			: undefined;
	if (said === 'opening') {
		return { kind: 'opening' };
	}
	return said === 'signed_out' ? { kind: 'signedOut' } : { kind: 'done' };
}

/** Ask this computer to forget the session it holds for one marketplace.
 *
 * Half of the seller's disconnect and the half that runs first, because until
 * it does the machine still holds cookies for a marketplace the server has
 * been told is disconnected. The other half is the control plane's own write,
 * which the caller performs whatever this answers. */
export async function forgetHere(
	invoke: Invoke | null,
	marketplace: Marketplace
): Promise<SessionOutcome> {
	return ranHere(invoke, FORGET_SESSION, marketplace, FORGET_REFUSED_SILENTLY);
}

/** The plugin command that hands one address to the browser this computer
 *  already uses.
 *
 * Not one of the application's own commands: `plugin:` names a Tauri plugin
 * and `|` separates it from the command, which is how Tauri routes a plugin
 * call. Written here for the reason `START_IMPORT` is — the string the console
 * sends and the grant the application carries are compared in one place, by
 * `desktop.test.ts` against `capabilities/opener.json`. */
export const OPEN_URL = 'plugin:opener|open_url';

/** What asking this computer to open an address produced.
 *
 * `unavailable` is every browser, where nothing was asked and nothing failed:
 * the caller lets the anchor do what an anchor does. `refused` carries the
 * application's own sentence, which no seller is shown — a refusal here is a
 * fault in our own grant, has no seller remedy, and the caller falls back to
 * what the link did before this existed. The sentence is carried anyway so a
 * test can tell a refusal apart from a success. */
export type OpenOutcome =
	| { kind: 'opened' }
	| { kind: 'refused'; detail: string }
	| { kind: 'unavailable' };

/** Ask this computer to open one address in the seller's own browser.
 *
 * The address goes to the platform's URL handler and nothing about it is
 * composed here. Nothing is fetched: this console never contacts a
 * marketplace, and neither does the application — the browser the seller
 * already uses does, under whatever session it already holds. */
export async function openExternal(invoke: Invoke | null, url: string): Promise<OpenOutcome> {
	if (invoke === null) {
		return { kind: 'unavailable' };
	}
	try {
		await invoke(OPEN_URL, { url });
		return { kind: 'opened' };
	} catch (caught) {
		return {
			kind: 'refused',
			detail: refusalText(caught, OPEN_REFUSED_SILENTLY)
		};
	}
}

/** The two marketplace commands' shared body: both take one marketplace, both
 *  answer nothing this console reads, and both classify a rejection the same
 *  three ways.
 *
 * The argument key is `marketplace` because that is what the application's own
 * command signature calls it, and Tauri matches an argument by name. */
async function ranHere(
	invoke: Invoke | null,
	command: string,
	marketplace: Marketplace,
	fallback: string
): Promise<SessionOutcome> {
	if (invoke === null) {
		return { kind: 'unavailable' };
	}
	try {
		await invoke(command, { marketplace });
		return { kind: 'done' };
	} catch (caught) {
		return refusal(caught, command, fallback);
	}
}

/** How a rejected marketplace command is classified, wherever one is called
 *  from. One place, so a connect and a forget cannot come to read the
 *  application's refusals differently. */
function refusal(caught: unknown, command: string, fallback: string): SessionOutcome {
	const detail = refusalText(caught, fallback);
	if (originNotGranted(detail, command)) {
		return { kind: 'refused', detail: ORIGIN_NOT_GRANTED };
	}
	return unknownCommand(detail) ? { kind: 'unsupported' } : { kind: 'refused', detail };
}

/** What a seller is told when this machine was signed out of their account.
 *
 * The same sentence `SIGNED_OUT_HERE` carries in
 * `apps/desktop/src-tauri/src/heartbeat.rs`, and the one a caller compares
 * against: a page that wants to stop offering an action a signed-out machine
 * cannot perform tests `detail === DEVICE_SIGNED_OUT` rather than reading a
 * message.
 *
 * The pre-press fact is `machineHere.revoked`, which is what a page should
 * gate a control on; this is the answer for the window between a sign-out and
 * this console's next check-in, and for a press that raced one. */
export const DEVICE_SIGNED_OUT =
	'This machine was signed out of your Teachouse account, so it cannot run imports. Sign it ' +
	'back in from Marketplaces, then try again.';

/** Whether a refusal is this machine having been signed out, rather than any
 *  other refusal the application or the server can give.
 *
 * Three spellings reach here and they are all the same fact. The application
 * refuses locally with `SIGNED_OUT_HERE`, which is `DEVICE_SIGNED_OUT`
 * verbatim. A press that raced the sign-out reaches the server, whose import
 * routes refuse with "this device is revoked and may not report a catalogue";
 * the desktop client turns that into its own typed answer, and this catches
 * the phrase for a client older than that change — which on Android was the
 * reported defect, a raw `403` body with JSON in it rendered to a teacher.
 *
 * Narrow on purpose. Nothing here matches a bare 403 or the word "forbidden",
 * because an ordinary refusal must keep saying what it said: telling a seller
 * their machine was signed out whenever a request was refused would be a false
 * accusation carrying the wrong remedy. */
function signedOut(detail: string): boolean {
	const said = detail.toLowerCase();
	return (
		said.includes('signed out of your teachouse account') ||
		said.includes('device is revoked') ||
		said.includes('device revoked')
	);
}

/** Whether the application rejected the call because it registers no such
 *  command, rather than because it considered the request and declined.
 *
 * Matched on the phrase the rejection ends with rather than on the whole
 * sentence, because the sentence now leads with the command's own name: a name
 * nothing registers was measured, against the context this application
 * generates, as exactly `a_command_nothing_registers not allowed. Command not
 * found`.
 *
 * The name leads because the access-control gate answers first. Now that
 * `apps/desktop/src-tauri/build.rs` declares an `AppManifest`, the gate at
 * `webview/mod.rs:1823-1850` of tauri 2.11.5 — the version
 * `apps/desktop/src-tauri/Cargo.toml` pins — fires for every command no
 * capability resolves, ahead of the dispatch at `:1911` that would otherwise
 * have rejected an unregistered command with `Command {name} not found` on its
 * own. The gate's message ends in that same phrase, assembled separately by
 * `resolve_access_message` in `ipc/authority.rs`, so matching the tail is what
 * makes the gate's spelling and the dispatch's one case.
 *
 * The gate composes that message only under `debug_assertions`; without them it
 * says `Command {name} not allowed by ACL` and this answer is false. The
 * workspace sets `debug-assertions = true` under `[profile.release]` in
 * `Cargo.toml`, so a shipped build takes the same branch a development one
 * does — which makes that profile line part of what this match rests on,
 * alongside the version, the build file and the capability.
 *
 * Reading a message is fragile and this is the only place the console does it,
 * so the failure is bounded: anything that changes the wording makes this
 * answer false and the outcome falls back to an ordinary refusal showing that
 * message verbatim, which is worse copy but never silence and never a wrong
 * claim. */
function unknownCommand(detail: string): boolean {
	return detail.trim().endsWith('Command not found');
}

/** Whether the application refused because the page asking is not one its
 *  capability grants, rather than because it considered the request.
 *
 * This arm is reached when a capability does grant the command but no grant
 * matches the asking origin, and the gate answers with the command's name, then
 * `not allowed on window`, then the window, the webview and the URL: measured
 * as `start_import not allowed on window "main", webview "main", URL:
 * http://tauri.localhost/`, followed by the grants and the permissions behind
 * them. Anchored at the front, because the tail is the part that enumerates
 * the fence and the part that grows.
 *
 * The command is a parameter rather than the one constant this file used to
 * hard-code, because the gate names whichever command was called and three of
 * them now reach it. Matching one name against another command's rejection
 * would fall through to the ordinary refusal and print the fence.
 *
 * A configuration fault or a development console, never a seller's doing. It is
 * separated from a refusal the application means only so the seller is not
 * shown the fence; both are refusals to them. */
function originNotGranted(detail: string, command: string): boolean {
	return detail.trim().startsWith(`${command} not allowed on window`);
}

/** The application's words, recovered from whatever the bridge rejected with.
 *
 * A rejected command carries `CommandError`, a newtype over a string, so the
 * ordinary case is already a string. The other arms exist so that a refusal is
 * never rendered as "[object Object]" or swallowed into an empty line: a
 * refusal the seller cannot read is the same as no refusal at all. */
function refusalText(caught: unknown, fallback: string): string {
	if (typeof caught === 'string' && caught.trim() !== '') {
		return caught;
	}
	if (caught instanceof Error && caught.message.trim() !== '') {
		return caught.message;
	}
	if (typeof caught === 'object' && caught !== null) {
		const message = (caught as { message?: unknown }).message;
		if (typeof message === 'string' && message.trim() !== '') {
			return message;
		}
	}
	return fallback;
}

/** The fallback each verb needs when the application rejects with nothing
 *  readable. Named per verb rather than written once and generalised, because
 *  a sentence that says "the command" names a thing the seller has no concept
 *  of. */
const IMPORT_REFUSED_SILENTLY = 'This computer refused to start the import and gave no reason.';
const CONNECT_REFUSED_SILENTLY = 'This computer refused to open the sign-in and gave no reason.';
const FORGET_REFUSED_SILENTLY =
	'This computer refused to remove the marketplace login and gave no reason.';
const OPEN_REFUSED_SILENTLY = 'This computer refused to open the link and gave no reason.';
const STOP_REFUSED_SILENTLY = 'This computer refused to stop the import and gave no reason.';
const SESSION_UNREADABLE =
	'This computer could not say whether it is signed in to that marketplace.';
