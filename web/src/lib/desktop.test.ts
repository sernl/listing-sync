import { afterEach, describe, expect, it, vi } from 'vitest';
import {
	APP_TOO_OLD,
	ORIGIN_NOT_GRANTED,
	START_IMPORT,
	desktopInvoker,
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

describe('asking this computer to run an import', () => {
	it('invokes the command with the request id', async () => {
		const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
		const invoke: Invoke = async (command, args) => {
			calls.push({ command, args });
			return null;
		};
		const outcome = await startImportHere(invoke, 'r-7');
		expect(outcome).toEqual({ kind: 'started' });
		expect(calls).toEqual([{ command: START_IMPORT, args: { request: 'r-7' } }]);
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
