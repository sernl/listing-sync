import { afterEach, describe, expect, it, vi } from 'vitest';
import {
	APP_CANNOT_CONNECT,
	APP_CANNOT_FORGET,
	APP_TOO_OLD,
	CONNECT_MARKETPLACE,
	CONTINUE_IMPORT,
	DEVICE_CHECK_IN,
	DEVICE_SIGNED_OUT,
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
	const MACHINE = { device_id: 'dev_9', device_name: 'Studio laptop' };

	it('invokes the check-in and says the registry may have gained a row', async () => {
		const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
		const invoke: Invoke = async (command, args) => {
			calls.push({ command, args });
			return { revoked: false, reached_server: true, signed_in: true, ...MACHINE };
		};
		expect(await checkInHere(invoke)).toEqual({
			reached: true,
			detail: null,
			revoked: false,
			device: { id: 'dev_9', name: 'Studio laptop' }
		});
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
		expect((await checkInHere(invoke)).reached).toBe(false);
	});

	it('says no row was gained when the application rejects the command', async () => {
		const invoke: Invoke = async (command) => {
			throw `${command} not allowed. Command not found`;
		};
		expect((await checkInHere(invoke)).reached).toBe(false);
	});

	it('does nothing at all in a browser, which is not a failure', async () => {
		expect(await checkInHere(null)).toEqual({
			reached: false,
			detail: null,
			revoked: false,
			device: null
		});
	});
});

describe('reading which machine the console is on', () => {
	// The founder's own ask: every platform must say which machine the seller
	// is at. The console is one build served to a browser and to the app
	// window around it, so this answer is the only thing that can tell them
	// apart.
	it('carries the identifier and the name the application sent', async () => {
		const invoke: Invoke = async () => ({
			reached_server: true,
			device_id: 'dev_1',
			device_name: 'SM-N975F'
		});
		expect((await checkInHere(invoke)).device).toEqual({ id: 'dev_1', name: 'SM-N975F' });
	});

	// An application older than the two fields, a half-populated answer, and a
	// browser all mean the same thing: this console cannot name the machine.
	// Naming half of one is worse — the restore route addresses a device by
	// id, so a name with no id is a button that cannot be pressed.
	it('names no machine unless both halves arrived', async () => {
		for (const answer of [
			{ reached_server: true },
			{ reached_server: true, device_id: 'dev_1' },
			{ reached_server: true, device_name: 'SM-N975F' },
			{ reached_server: true, device_id: '', device_name: 'SM-N975F' },
			{ reached_server: true, device_id: 'dev_1', device_name: '' },
			{ reached_server: true, device_id: 7, device_name: 'SM-N975F' }
		]) {
			expect((await checkInHere(async () => answer)).device, JSON.stringify(answer)).toBeNull();
		}
	});
});

describe('reading whether this machine was signed out from the console', () => {
	it('reports the revocation the heartbeat was told about', async () => {
		const invoke: Invoke = async () => ({ reached_server: true, revoked: true });
		expect((await checkInHere(invoke)).revoked).toBe(true);
	});

	// The fact the banner is raised on, so it may never be a guess. A machine
	// that could not reach the control plane learnt nothing about whether the
	// seller signed it out, and a banner raised over a dropped connection
	// would accuse them of an act they did not perform.
	it('claims no revocation from a check-in that never reached us', async () => {
		const invoke: Invoke = async () => ({ reached_server: false, revoked: true });
		expect((await checkInHere(invoke)).revoked).toBe(false);
	});

	it('reads an application too old to send the flag as not revoked', async () => {
		const invoke: Invoke = async () => ({ reached_server: true });
		expect((await checkInHere(invoke)).revoked).toBe(false);
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
		expect(await checkInHere(invoke)).toEqual({
			reached: true,
			detail: null,
			revoked: false,
			device: null
		});
	});

	it("carries the application's own sentence when it did not", async () => {
		const invoke: Invoke = async () => ({
			revoked: false,
			reached_server: false,
			signed_in: false,
			detail: 'this device is not registered'
		});
		expect((await checkInHere(invoke)).detail).toBe('this device is not registered');
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
		expect((await checkInHere(invoke)).detail).toBeNull();
	});

	it('carries no reason when the application rejects the command', async () => {
		const invoke: Invoke = async (command) => {
			throw `${command} not allowed. Command not found`;
		};
		expect((await checkInHere(invoke)).detail).toBeNull();
	});

	it('sends one command and nothing at all in a browser', async () => {
		const calls: string[] = [];
		const invoke: Invoke = async (command) => {
			calls.push(command);
			return { reached_server: true };
		};
		await checkInHere(invoke);
		expect(calls).toEqual([DEVICE_CHECK_IN]);
		expect((await checkInHere(null)).reached).toBe(false);
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
		//
		// `takeover` travels with it and is false unless the seller has said
		// otherwise: an ordinary press must never take a run away from a phone
		// that is reading a shop right now.
		expect(calls).toEqual([
			{ command: START_IMPORT, args: { run: 'r-7', takeover: false } },
			{ command: CONTINUE_IMPORT, args: { run: 'r-7', takeover: false } }
		]);
	});

	it('carries a confirmed takeover, and only when it was asked for', async () => {
		const calls: Array<Record<string, unknown>> = [];
		const invoke: Invoke = async (_command, args) => {
			calls.push(args);
			return null;
		};
		await startImportHere(invoke, 'r-7', true);
		await continueImportHere(invoke, 'r-7', true);
		expect(calls).toEqual([
			{ run: 'r-7', takeover: true },
			{ run: 'r-7', takeover: true }
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

	// The Android defect, as a contract. A device the seller signed out was
	// refused by the server, and the forbidden answer's JSON body was rendered
	// to a teacher verbatim. Every spelling of that one fact now becomes the
	// same sentence, and a caller can compare against it to stop offering an
	// action the machine cannot perform.
	it('turns every spelling of a signed-out machine into one teacher-facing sentence', async () => {
		for (const raw of [
			DEVICE_SIGNED_OUT,
			'This machine was signed out of your Teachouse account, so it cannot run imports. Sign it back in from Marketplaces, then try again.',
			'the control plane refused: 403: {"errors":[{"message":"this device is revoked and may not report a catalogue","kind":"validation"}]}',
			'403 device revoked'
		]) {
			const invoke: Invoke = async () => {
				throw raw;
			};
			const outcome = await startImportHere(invoke, 'r-7');
			expect(outcome).toEqual({ kind: 'refused', detail: DEVICE_SIGNED_OUT });
			const detail = outcome.kind === 'refused' ? outcome.detail : '';
			expect(detail).not.toContain('{');
			expect(detail).not.toContain('403');
			expect(detail).toContain('Sign it back in');
		}
	});

	// And nothing else becomes it. A forbidden answer about a lease, a plan or
	// a bare status is a different fact with a different remedy; reading one as
	// a sign-out would accuse the seller of an act they did not perform and
	// send them to a restore they do not need.
	it('leaves every other refusal, forbidden ones included, exactly as it was', async () => {
		for (const sentence of [
			'the server refused this device\'s sign-in: 403: {"errors":[{"message":"this device holds no live lease on an item whose projection names that file"}]}',
			'the control plane refused: 403: Forbidden',
			'your subscription does not cover imports'
		]) {
			const invoke: Invoke = async () => {
				throw sentence;
			};
			expect(await startImportHere(invoke, 'r-7')).toEqual({
				kind: 'refused',
				detail: sentence
			});
		}
	});

	// Measured from a console served somewhere the capability does not name: the
	// application answers with the window, the webview, every granted origin and
	// the permission behind each grant.
	const WRONG_ORIGIN_REJECTION = [
		`${START_IMPORT} not allowed on window "main", webview "main", URL: http://tauri.localhost/`,
		'',
		'allowed on: [windows: "main", URL: local], [windows: "main", URL: https://teachouse.io]',
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
			'teachouse.io',
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
					'allowed on: [windows: "main", URL: local], [windows: "main", URL: https://teachouse.io]',
					'',
					`referenced by: capability: console, permission: allow-${command.replace(/_/g, '-')}`
				].join('\n');
			};
			const outcome = await run(invoke, 'Tpt');
			expect(outcome, command).toEqual({ kind: 'refused', detail: ORIGIN_NOT_GRANTED });
			const detail = outcome.kind === 'refused' ? outcome.detail : '';
			for (const internal of [
				'teachouse.io',
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

	// The refusal in front of the password, on both surfaces. The application
	// checks in before it opens anything and answers this instead where the
	// machine has been signed out from the console — a capture it took would
	// be wiped by the same check-in, which is exactly what the founder met on
	// 0.7.0: he typed a TES password on a machine whose session was thrown
	// away, three times.
	it('reads the signed-out answer as its own outcome rather than as done', async () => {
		const invoke = vi.fn().mockResolvedValue({ outcome: 'signed_out' });
		expect(await connectHere(invoke, 'Tes')).toEqual({ kind: 'signedOut' });
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
