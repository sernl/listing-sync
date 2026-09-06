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

/** The desktop command that runs one import on this computer.
 *
 * Named here rather than written at the call site so the string the console
 * sends and the string the application registers are compared in one place. */
export const START_IMPORT = 'start_import';

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

/** Register this machine with the server, from the console running inside it.
 *
 * The application registers itself on its own scheduled cycle as well, and this
 * is the half that makes it prompt. That cycle runs at start-up, which is before
 * the seller has signed in and therefore has no session to register under, and
 * then hourly on a computer or on the next resume on a phone — so without this
 * call a seller who has just signed in looks at an empty machine list and has no
 * way to tell a slow registration from a broken one.
 *
 * Answers whether the registry may now have gained a row, which is the caller's
 * cue to read it again. False in every browser, where there is no application to
 * register and nothing failed. False also when the application could not reach
 * the server: the command reports that as an ordinary answer rather than a
 * rejection, and a list refetched over the same dead connection would only fail
 * twice. */
export async function registerThisMachine(invoke: Invoke | null): Promise<boolean> {
	if (invoke === null) {
		return false;
	}
	try {
		return reachedServer(await invoke(DEVICE_CHECK_IN, {}));
	} catch {
		// Every rejection alike: an application too old to know the command, a
		// page the application takes no commands from, and a genuine refusal all
		// mean the same thing here, which is that no row was added. There is
		// nothing to tell the seller, because they did not ask for this.
		return false;
	}
}

/** Whether a check-in answer says the application reached the server.
 *
 * `reached_server` is one field of the application's `DeviceState`, and the only
 * one this caller reads: `revoked` and `signed_in` are shown by the machine list
 * itself, out of the registry, which is the copy the seller can act on. */
function reachedServer(answer: unknown): boolean {
	return (
		typeof answer === 'object' &&
		answer !== null &&
		(answer as { reached_server?: unknown }).reached_server === true
	);
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

/** Ask this computer to run one import.
 *
 * `unavailable` where there is no invoker, which is every browser: the caller
 * shows the seller how to run it on the machine that holds the session rather
 * than reporting a failure, because nothing failed. */
export async function startImportHere(
	invoke: Invoke | null,
	request: string
): Promise<StartOutcome> {
	if (invoke === null) {
		return { kind: 'unavailable' };
	}
	try {
		await invoke(START_IMPORT, { request });
		return { kind: 'started' };
	} catch (caught) {
		const detail = refusalText(caught, IMPORT_REFUSED_SILENTLY);
		if (originNotGranted(detail, START_IMPORT)) {
			return { kind: 'refused', detail: ORIGIN_NOT_GRANTED };
		}
		return unknownCommand(detail) ? { kind: 'unsupported' } : { kind: 'refused', detail };
	}
}

/** What asking this computer to connect or forget one marketplace produced.
 *
 * The same four answers `StartOutcome` carries, with `done` where that one has
 * `started`: a connect either completed on this machine or it did not, and
 * there is no third thing running afterwards for a seller to wait on.
 *
 * `unavailable` is the ordinary browser answer and is not a failure. The
 * caller shows the seller where the act can be performed instead. */
export type SessionOutcome =
	| { kind: 'done' }
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
	return ranHere(invoke, CONNECT_MARKETPLACE, marketplace, CONNECT_REFUSED_SILENTLY);
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
		return { kind: 'refused', detail: refusalText(caught, OPEN_REFUSED_SILENTLY) };
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
		const detail = refusalText(caught, fallback);
		if (originNotGranted(detail, command)) {
			return { kind: 'refused', detail: ORIGIN_NOT_GRANTED };
		}
		return unknownCommand(detail) ? { kind: 'unsupported' } : { kind: 'refused', detail };
	}
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
