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
		const detail = refusalText(caught);
		if (originNotGranted(detail)) {
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
 * A configuration fault or a development console, never a seller's doing. It is
 * separated from a refusal the application means only so the seller is not
 * shown the fence; both are refusals to them. */
function originNotGranted(detail: string): boolean {
	return detail.trim().startsWith(`${START_IMPORT} not allowed on window`);
}

/** The application's words, recovered from whatever the bridge rejected with.
 *
 * A rejected command carries `CommandError`, a newtype over a string, so the
 * ordinary case is already a string. The other arms exist so that a refusal is
 * never rendered as "[object Object]" or swallowed into an empty line: a
 * refusal the seller cannot read is the same as no refusal at all. */
function refusalText(caught: unknown): string {
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
	return 'This computer refused to start the import and gave no reason.';
}
