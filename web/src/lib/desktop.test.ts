import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
	APP_CANNOT_CONNECT,
	APP_CANNOT_FORGET,
	APP_TOO_OLD,
	CONNECT_MARKETPLACE,
	CONTINUE_IMPORT,
	DEVICE_CHECK_IN,
	FORGET_SESSION,
	OPEN_URL,
	ORIGIN_NOT_GRANTED,
	START_IMPORT,
	checkInHere,
	connectHere,
	continueImportHere,
	desktopInvoker,
	forgetHere,
	openExternal,
	registerThisMachine,
	startImportHere
} from './desktop';
import type { Invoke } from './desktop';

afterEach(() => {
	vi.unstubAllGlobals();
});

describe('reaching the desktop application', () => {
	it('finds no invoker in an ordinary browser', () => {
		expect(desktopInvoker()).toBeNull();
	});

	it('finds none when the global is there but carries no invoke', () => {
		vi.stubGlobal('window', { __TAURI__: { core: {} } });
		expect(desktopInvoker()).toBeNull();
		vi.stubGlobal('window', { __TAURI__: {} });
		expect(desktopInvoker()).toBeNull();
	});

	it('finds the application invoker when it is exposed', () => {
		const invoke = vi.fn();
		vi.stubGlobal('window', { __TAURI__: { core: { invoke } } });
		expect(desktopInvoker()).toBe(invoke);
	});
});

describe('putting this machine in the registry', () => {
	it('invokes the check-in and says the registry may have gained a row', async () => {
		const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
		const invoke: Invoke = async (command, args) => {
			calls.push({ command, args });
			return { revoked: false, reached_server: true, signed_in: true };
		};
		expect(await registerThisMachine(invoke)).toBe(true);
		expect(calls).toEqual([{ command: DEVICE_CHECK_IN, args: {} }]);
	});

	it('says no row was gained when the application could not reach the server', async () => {
		// The application answers rather than rejecting here, so a caller that
		// read the call's success would refetch over a connection it has just
		// been told is dead.
		const invoke: Invoke = async () => ({
			revoked: false,
			reached_server: false,
			signed_in: false
		});
		expect(await registerThisMachine(invoke)).toBe(false);
	});

	it('says no row was gained when the application rejects the command', async () => {
		const invoke: Invoke = async (command) => {
			throw `${command} not allowed. Command not found`;
		};
		expect(await registerThisMachine(invoke)).toBe(false);
	});

	it('does nothing at all in a browser, which is not a failure', async () => {
		expect(await registerThisMachine(null)).toBe(false);
	});
});

describe('reading why a check-in did not reach us', () => {
	it('carries no reason when it did reach', async () => {
		const invoke: Invoke = async () => ({
			revoked: false,
			reached_server: true,
			signed_in: true,
			detail: null
		});
		expect(await checkInHere(invoke)).toEqual({ reached: true, detail: null });
	});

	it("carries the application's own sentence when it did not", async () => {
		const invoke: Invoke = async () => ({
			revoked: false,
			reached_server: false,
			signed_in: false,
			detail: 'this device is not registered'
		});
		expect(await checkInHere(invoke)).toEqual({
			reached: false,
			detail: 'this device is not registered'
		});
	});

	it('carries no reason from an application too old to send one', async () => {
		// The console is served from the control plane and updates when we
		// deploy; the application updates when the seller takes an update. The
		// console is therefore the newer of the two, and this is that arm: the
		// field is simply absent, and absent must not read as a named cause.
		const invoke: Invoke = async () => ({
			revoked: false,
			reached_server: false,
			signed_in: false
		});
		expect(await checkInHere(invoke)).toEqual({ reached: false, detail: null });
	});

	it('carries no reason when the application rejects the command', async () => {
		const invoke: Invoke = async (command) => {
			throw `${command} not allowed. Command not found`;
		};
		expect(await checkInHere(invoke)).toEqual({ reached: false, detail: null });
	});

	it('sends the same command registerThisMachine does, and nothing in a browser', async () => {
		const calls: string[] = [];
		const invoke: Invoke = async (command) => {
			calls.push(command);
			return { reached_server: true };
		};
		await checkInHere(invoke);
		expect(calls).toEqual([DEVICE_CHECK_IN]);
		expect(await checkInHere(null)).toEqual({ reached: false, detail: null });
	});
});

describe('asking this computer to run an import', () => {
	it('invokes each half of an import with the run it addresses', async () => {
		const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
		const invoke: Invoke = async (command, args) => {
			calls.push({ command, args });
			return null;
		};
		expect(await startImportHere(invoke, 'r-7')).toEqual({ kind: 'started' });
		expect(await continueImportHere(invoke, 'r-7')).toEqual({
			kind: 'started'
		});
		// The argument key is what Tauri matches a parameter by, so the two
		// halves send the run under the name the commands declare and nothing
		// else. A page that still sent `request` would be answered by an
		// application that saw no run at all.
		expect(calls).toEqual([
			{ command: START_IMPORT, args: { run: 'r-7' } },
			{ command: CONTINUE_IMPORT, args: { run: 'r-7' } }
		]);
	});

	// Replaces an assertion that compared START_IMPORT to a second copy of the
	// same literal. That compared the constant to itself and passed while no
	// application registered the command at all, which is the one condition it
	// mattered under. What matters is not the string's spelling but what the
	// console does when the application does not know it.
	//
	// The rejection is the one measured against the context this application
	// generates, now that it declares a manifest: the access-control gate answers
	// ahead of dispatch, so the command's own name leads and the phrase the
	// console reads is what the sentence ends with.
	it('tells the seller to update when the application does not know the command', async () => {
		const invoke: Invoke = async (command) => {
			throw `${command} not allowed. Command not found`;
		};
		const outcome = await startImportHere(invoke, 'r-7');
		expect(outcome).toEqual({ kind: 'unsupported' });
	});

	it('an unsupported application is not reported as a refusal, and reads as an update', async () => {
		const invoke: Invoke = async () => {
			throw `${START_IMPORT} not allowed. Command not found`;
		};
		expect((await startImportHere(invoke, 'r-7')).kind).not.toBe('refused');
		expect(APP_TOO_OLD).toMatch(/update/i);
		// Never Tauri's own words, which name a command a seller has no concept of.
		expect(APP_TOO_OLD).not.toContain(START_IMPORT);
		expect(APP_TOO_OLD).not.toMatch(/command/i);
	});

	it('a considered refusal is still a refusal, not mistaken for an old application', async () => {
		for (const sentence of [
			'no Tes session is held on this computer for that shop',
			'your subscription does not cover imports',
			'a pass is already running for that request',
			'Command something_else not found'
		]) {
			const invoke: Invoke = async () => {
				throw sentence;
			};
			const outcome = await startImportHere(invoke, 'r-7');
			expect(outcome).toEqual({ kind: 'refused', detail: sentence });
		}
	});

	// The match is on the whole trailing phrase and nothing weaker. `not found`
	// alone is ordinary English a device may reach for about a listing or a shop,
	// and the wording this file matched before the manifest existed — `Command
	// {name} not found`, straight from dispatch — puts the name where the phrase
	// would have to be. Neither is an application saying it has no such command,
	// and both fall through to the refusal they are.
	it('reads only the trailing phrase, not a stray "not found" and not the old wording', async () => {
		for (const sentence of [
			`Command ${START_IMPORT} not found`,
			'Command something_else not found',
			'that listing was not found in your Tes shop, so nothing was imported',
			'the session was not found on this computer'
		]) {
			const invoke: Invoke = async () => {
				throw sentence;
			};
			expect(await startImportHere(invoke, 'r-7')).toEqual({ kind: 'refused', detail: sentence });
		}
		const ending: Invoke = async () => {
			throw '   something_else not allowed. Command not found   ';
		};
		expect(await startImportHere(ending, 'r-7')).toEqual({ kind: 'unsupported' });
	});

	// Measured from a console served somewhere the capability does not name: the
	// application answers with the window, the webview, every granted origin and
	// the permission behind each grant.
	const WRONG_ORIGIN_REJECTION = [
		`${START_IMPORT} not allowed on window "main", webview "main", URL: http://tauri.localhost/`,
		'',
		'allowed on: [windows: "main", URL: local], [windows: "main", URL: https://teachouse.stowiq.io]',
		'',
		'referenced by: capability: default, permission: allow-start-import'
	].join('\n');

	it('names an ungranted page as such, in words a seller can read', async () => {
		const invoke: Invoke = async () => {
			throw WRONG_ORIGIN_REJECTION;
		};
		const outcome = await startImportHere(invoke, 'r-7');
		expect(outcome).toEqual({ kind: 'refused', detail: ORIGIN_NOT_GRANTED });
	});

	it('never shows the fence: no origin, permission or window reaches the seller', async () => {
		const invoke: Invoke = async () => {
			throw WRONG_ORIGIN_REJECTION;
		};
		const outcome = await startImportHere(invoke, 'r-7');
		const detail = outcome.kind === 'refused' ? outcome.detail : '';
		for (const internal of [
			'teachouse.stowiq.io',
			'tauri.localhost',
			'capability',
			'permission',
			'webview',
			'window',
			'allowed on',
			START_IMPORT
		]) {
			expect(detail).not.toContain(internal);
		}
		expect(detail.trim()).not.toBe('');
	});

	it('an ungranted page is neither a started import nor an old application', async () => {
		const invoke: Invoke = async () => {
			throw WRONG_ORIGIN_REJECTION;
		};
		const outcome = await startImportHere(invoke, 'r-7');
		expect(outcome.kind).not.toBe('started');
		expect(outcome.kind).not.toBe('unsupported');
		// An update is not the remedy, so it must not read as one.
		expect(ORIGIN_NOT_GRANTED).not.toMatch(/update/i);
	});

	it('invokes nothing at all in a browser, and calls that unavailable', async () => {
		const outcome = await startImportHere(null, 'r-7');
		expect(outcome).toEqual({ kind: 'unavailable' });
	});

	it("shows the application's refusal verbatim rather than swallowing it", async () => {
		const invoke: Invoke = async () => {
			throw 'no Tes session is held on this computer for that shop';
		};
		const outcome = await startImportHere(invoke, 'r-7');
		expect(outcome).toEqual({
			kind: 'refused',
			detail: 'no Tes session is held on this computer for that shop'
		});
	});

	it('reads a refusal that arrives as an Error rather than a string', async () => {
		const invoke: Invoke = async () => {
			throw new Error('a pass is already running for that request');
		};
		const outcome = await startImportHere(invoke, 'r-7');
		expect(outcome).toEqual({
			kind: 'refused',
			detail: 'a pass is already running for that request'
		});
	});

	it('never renders a refusal as an empty line or an object', async () => {
		for (const thrown of [undefined, null, '', '   ', {}, { message: '' }]) {
			const invoke: Invoke = async () => {
				throw thrown;
			};
			const outcome = await startImportHere(invoke, 'r-7');
			expect(outcome.kind).toBe('refused');
			const detail = outcome.kind === 'refused' ? outcome.detail : '';
			expect(detail.trim()).not.toBe('');
			expect(detail).not.toContain('[object');
		}
	});

	it('a refused start is never reported as a started one', async () => {
		const invoke: Invoke = async () => {
			throw 'entitlement closed';
		};
		expect((await startImportHere(invoke, 'r-7')).kind).not.toBe('started');
	});
});

describe('asking this computer to connect or forget one marketplace', () => {
	it('invokes each command with the marketplace under the name the app expects', async () => {
		const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
		const invoke: Invoke = async (command, args) => {
			calls.push({ command, args });
			return null;
		};
		expect(await connectHere(invoke, 'Tpt')).toEqual({ kind: 'done' });
		expect(await forgetHere(invoke, 'Tes')).toEqual({ kind: 'done' });
		// The key is `marketplace` because Tauri matches an argument by name and
		// the command signature spells it that way; a renamed key is a rejection
		// no type would catch.
		expect(calls).toEqual([
			{ command: CONNECT_MARKETPLACE, args: { marketplace: 'Tpt' } },
			{ command: FORGET_SESSION, args: { marketplace: 'Tes' } }
		]);
	});

	it('invokes nothing at all in a browser, and calls that unavailable', async () => {
		expect(await connectHere(null, 'Tpt')).toEqual({ kind: 'unavailable' });
		expect(await forgetHere(null, 'Tpt')).toEqual({ kind: 'unavailable' });
	});

	it('tells the seller to update when the application does not know the command', async () => {
		const invoke: Invoke = async (command) => {
			throw `${command} not allowed. Command not found`;
		};
		expect(await connectHere(invoke, 'Tpt')).toEqual({ kind: 'unsupported' });
		expect(await forgetHere(invoke, 'Tpt')).toEqual({ kind: 'unsupported' });
	});

	it('words each update remedy for the act the seller pressed, never for an import', async () => {
		for (const sentence of [APP_CANNOT_CONNECT, APP_CANNOT_FORGET]) {
			expect(sentence).not.toContain('import');
			expect(sentence).not.toMatch(/command/i);
		}
		expect(APP_CANNOT_CONNECT).toMatch(/update/i);
		// The one that must also say what still stands: a disconnect the machine
		// could not perform leaves the login exactly where it was.
		expect(APP_CANNOT_FORGET).toContain('still on');
		expect(APP_TOO_OLD).toContain('import');
	});

	// The substitution is per command, because the gate names whichever command
	// was called. Matching one name against another command's rejection would
	// fall through to an ordinary refusal and print the fence verbatim.
	it('names an ungranted page as such for each command, and shows the seller no fence', async () => {
		for (const [command, run] of [
			[CONNECT_MARKETPLACE, connectHere],
			[FORGET_SESSION, forgetHere]
		] as const) {
			const invoke: Invoke = async () => {
				throw [
					`${command} not allowed on window "main", webview "main", URL: http://tauri.localhost/`,
					'',
					'allowed on: [windows: "main", URL: local], [windows: "main", URL: https://teachouse.stowiq.io]',
					'',
					`referenced by: capability: console, permission: allow-${command.replace(/_/g, '-')}`
				].join('\n');
			};
			const outcome = await run(invoke, 'Tpt');
			expect(outcome, command).toEqual({ kind: 'refused', detail: ORIGIN_NOT_GRANTED });
			const detail = outcome.kind === 'refused' ? outcome.detail : '';
			for (const internal of [
				'teachouse.stowiq.io',
				'tauri.localhost',
				'capability',
				'permission',
				'webview',
				'allowed on',
				command
			]) {
				expect(detail, `${command}/${internal}`).not.toContain(internal);
			}
		}
	});

	it("shows the application's own refusal verbatim rather than interpreting it", async () => {
		// Every sentence `connect_marketplace` refuses with, as the command
		// itself words them. None is a code the console could branch on.
		for (const sentence of [
			'a login window for this marketplace is already open',
			'the login window was closed before the sign-in completed',
			"the sign-in did not complete before the window's deadline",
			'this device has been signed out from the console, so the marketplace session was not kept'
		]) {
			const invoke: Invoke = async () => {
				throw sentence;
			};
			expect(await connectHere(invoke, 'Tpt')).toEqual({ kind: 'refused', detail: sentence });
		}
	});

	it('never renders a refusal as an empty line or an object, and names the act it was', async () => {
		for (const thrown of [undefined, null, '', '   ', {}, { message: '' }]) {
			const invoke: Invoke = async () => {
				throw thrown;
			};
			const connect = await connectHere(invoke, 'Tpt');
			const forget = await forgetHere(invoke, 'Tpt');
			for (const outcome of [connect, forget]) {
				expect(outcome.kind).toBe('refused');
				const detail = outcome.kind === 'refused' ? outcome.detail : '';
				expect(detail.trim()).not.toBe('');
				expect(detail).not.toContain('[object');
			}
			// Each fallback names its own act, so a seller reading one after
			// pressing the other would notice.
			expect(connect.kind === 'refused' ? connect.detail : '').toContain('sign-in');
			expect(forget.kind === 'refused' ? forget.detail : '').toContain('login');
		}
	});

	it('a refused connect is never reported as a completed one', async () => {
		const invoke: Invoke = async () => {
			throw 'the login window was closed before the sign-in completed';
		};
		expect((await connectHere(invoke, 'Tpt')).kind).not.toBe('done');
	});
});

describe('opening a marketplace link in the seller\'s own browser', () => {
	it('asks nothing in a browser, where the anchor already does the right thing', async () => {
		const invoke = vi.fn();
		expect(await openExternal(null, 'https://www.tes.com/teaching-resource/x-1')).toEqual({
			kind: 'unavailable'
		});
		expect(invoke).not.toHaveBeenCalled();
	});

	it('hands the address to the opener plugin under the name the grant carries', async () => {
		const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
		const invoke: Invoke = async (command, args) => {
			calls.push({ command, args });
			return null;
		};
		const url = 'https://www.teacherspayteachers.com/Product/x-1';
		expect(await openExternal(invoke, url)).toEqual({ kind: 'opened' });
		expect(calls).toEqual([{ command: OPEN_URL, args: { url } }]);
	});

	it("carries the application's own words when it refuses", async () => {
		const invoke: Invoke = async () => {
			throw 'Forbidden URL';
		};
		expect(await openExternal(invoke, 'https://www.tes.com/')).toEqual({
			kind: 'refused',
			detail: 'Forbidden URL'
		});
	});

	it('never reports a refusal as an opened link, whatever it rejected with', async () => {
		for (const thrown of [undefined, null, '', '   ', {}, { message: '' }]) {
			const invoke: Invoke = async () => {
				throw thrown;
			};
			const outcome = await openExternal(invoke, 'https://www.tes.com/');
			expect(outcome.kind).toBe('refused');
			const detail = outcome.kind === 'refused' ? outcome.detail : '';
			expect(detail.trim()).not.toBe('');
			expect(detail).not.toContain('[object');
		}
	});
});

/** The application's own capability files, read from source.
 *
 * The Rust side already asserts that the origin the capability grants is the
 * one this build talks to (`control_plane.rs`,
 * `the_capability_grants_the_origin_this_build_uses`). Nothing asserted the
 * other direction: that the console's command string and the grant behind it
 * are one decision. A capability naming a permission the console never calls,
 * or a console calling a plugin the capability never grants, builds clean and
 * refuses at run time in front of a seller. */
function capability(name: string): Record<string, unknown> {
	return JSON.parse(
		readFileSync(
			fileURLToPath(
				new URL(`../../../apps/desktop/src-tauri/capabilities/${name}.json`, import.meta.url)
			),
			'utf8'
		)
	) as Record<string, unknown>;
}

describe('the grant behind the opener command', () => {
	const opener = capability('opener');

	it('grants open_url for the plugin the console names', () => {
		// `plugin:opener|open_url` is Tauri's own routing: the plugin, then the
		// command. Both halves are read out rather than compared to a copy of
		// the same literal, so a rename on either side fails here.
		const [prefixed, command] = OPEN_URL.split('|');
		expect(prefixed).toBe('plugin:opener');
		const plugin = prefixed.slice('plugin:'.length);
		const granted = (opener.permissions as Array<{ identifier: string }>).map(
			(entry) => entry.identifier
		);
		expect(granted).toEqual([`${plugin}:allow-${command.replaceAll('_', '-')}`]);
	});

	it('carries a scope, without which the grant refuses every address', () => {
		// Not decoration on the permission. `allow-open-url` arrives with an
		// empty allow list, and tauri-plugin-opener 2.5.5 answers
		// `is_url_allowed` with `self.allowed.iter().any(..)` (src/scope.rs), so
		// the bare permission is a grant to a command that refuses everything.
		const scope = (opener.permissions as Array<{ allow?: Array<{ url?: string }> }>)[0].allow;
		expect(scope).toEqual([{ url: 'https://*' }]);
	});

	it('reaches the same origin the console is served from and no other', () => {
		// The console is one page whichever file grants it, so a grant that
		// named a second origin would widen the application's surface without
		// widening anything the console can do with it.
		const remote = (name: string) =>
			(capability(name).remote as { urls: string[] } | undefined)?.urls;
		expect(remote('opener')).toEqual(remote('console'));
		expect(remote('opener')).toEqual(['https://teachouse.stowiq.io']);
	});

	it('is granted to the console window only, so a login webview gains nothing', () => {
		expect(opener.windows).toEqual(['main']);
	});

	it('carries no platform list, unlike the updater it sits beside', () => {
		// Android is the platform this matters most on: its webview has no
		// second window, so without the plugin a marketplace link replaces the
		// console itself.
		expect(opener.platforms).toBeUndefined();
		expect(capability('updater').platforms).toEqual(['linux', 'macOS', 'windows']);
	});
});

describe('reading what a connect answered', () => {
	// The whole of the phone's connect path, and the case a computer never
	// produces: the application says the sign-in is replacing this page, and
	// the caller has to know not to say anything, because the page it would say
	// it on is about to unload.
	it('reads the opening answer a phone gives', async () => {
		const invoke = vi.fn().mockResolvedValue({ outcome: 'opening' });
		expect(await connectHere(invoke, 'Tpt')).toEqual({ kind: 'opening' });
		expect(invoke).toHaveBeenCalledWith(CONNECT_MARKETPLACE, { marketplace: 'Tpt' });
	});

	it('reads the captured answer a computer gives as done', async () => {
		const invoke = vi.fn().mockResolvedValue({
			outcome: 'captured',
			session: { marketplace: 'Tpt', connected: true }
		});
		expect(await connectHere(invoke, 'Tpt')).toEqual({ kind: 'done' });
	});

	// The compatibility arm, and it is the one that must not be an oversight:
	// before `ConnectOutcome` existed the command answered a bare session
	// status, which carries no `outcome` at all and meant a completed capture.
	// An application older than this console has to read as the success it was,
	// not as a state the console then waits in forever.
	it('reads an older application answer as done rather than as opening', async () => {
		for (const answer of [
			{ marketplace: 'Tpt', connected: true, cookie_count: 2 },
			{},
			null,
			undefined,
			'anything at all'
		]) {
			const invoke = vi.fn().mockResolvedValue(answer);
			expect(await connectHere(invoke, 'Tpt'), JSON.stringify(answer)).toEqual({
				kind: 'done'
			});
		}
	});

	it('still classifies every refusal the way a forget does', async () => {
		const refused = vi.fn().mockRejectedValue('this device has been signed out');
		expect(await connectHere(refused, 'Tpt')).toEqual({
			kind: 'refused',
			detail: 'this device has been signed out'
		});
		const missing = vi
			.fn()
			.mockRejectedValue('connect_marketplace not allowed. Command not found');
		expect(await connectHere(missing, 'Tpt')).toEqual({ kind: 'unsupported' });
		const ungranted = vi
			.fn()
			.mockRejectedValue('connect_marketplace not allowed on window "main", webview "main"');
		expect(await connectHere(ungranted, 'Tpt')).toEqual({
			kind: 'refused',
			detail: ORIGIN_NOT_GRANTED
		});
		expect(await connectHere(null, 'Tpt')).toEqual({ kind: 'unavailable' });
	});
});
