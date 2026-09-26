import { describe, expect, it, vi } from 'vitest';
import { ApiFailure } from '$lib/api';
import {
	FALLBACK_FILENAME,
	HANDOFF_FAILED,
	REFUSED_WITHOUT_REASON,
	SESSION_ENDED,
	failureMessage,
	failureOf,
	filenameFrom,
	runExport,
	type CsvDocument
} from './download';

const SERVER_NAME = 'teachouse-resources-2026-09-05.csv';

function csvDocument(filename = SERVER_NAME): CsvDocument {
	return { blob: new Blob(['Title\r\n']), filename };
}

describe('the name the file is saved under', () => {
	it('is the one the server put in the header', () => {
		expect(filenameFrom(`attachment; filename="${SERVER_NAME}"`)).toBe(SERVER_NAME);
	});

	it('reads an unquoted name, and a header whose parameters are spaced out', () => {
		expect(filenameFrom(`attachment; filename=${SERVER_NAME}`)).toBe(SERVER_NAME);
		expect(filenameFrom(`attachment ; filename = "${SERVER_NAME}" ; foo=1`)).toBe(SERVER_NAME);
	});

	it('trims an unquoted name that is padded before the next parameter', () => {
		expect(filenameFrom('attachment; filename=x.csv ; foo=1')).toBe('x.csv');
	});

	it('falls back, undated, when no header came back at all', () => {
		expect(filenameFrom(null)).toBe(FALLBACK_FILENAME);
	});

	it('falls back when the header carries no name, or an empty one', () => {
		expect(filenameFrom('attachment')).toBe(FALLBACK_FILENAME);
		expect(filenameFrom('attachment; filename=""')).toBe(FALLBACK_FILENAME);
		expect(filenameFrom('attachment; filename="   "')).toBe(FALLBACK_FILENAME);
	});

	it('refuses a name carrying a path separator rather than cleaning it', () => {
		expect(filenameFrom('attachment; filename="../../etc/passwd"')).toBe(FALLBACK_FILENAME);
		expect(filenameFrom('attachment; filename="c:\\windows\\x.csv"')).toBe(FALLBACK_FILENAME);
	});

	it('takes the name only from a parameter actually called filename', () => {
		expect(filenameFrom('attachment; xfilename="evil.csv"')).toBe(FALLBACK_FILENAME);
	});

	it('never invents the date the server withheld', () => {
		expect(filenameFrom(null)).not.toMatch(/\d{4}-\d{2}-\d{2}/);
	});
});

describe('what the seller is told when no file arrives', () => {
	it('names signing in again when the session has gone', () => {
		expect(failureMessage(new ApiFailure(401, null))).toBe(SESSION_ENDED);
	});

	// The page shows a way to sign in, so the remedy travels with the sentence
	// rather than being recovered by matching on the words.
	it('offers signing in on the one refusal that has that remedy, and on no other', () => {
		expect(failureOf(new ApiFailure(401, null)).signIn).toBe(true);
		for (const caught of [
			new ApiFailure(403, null),
			new ApiFailure(404, null),
			new ApiFailure(429, { status: 429, errors: [{ message: 'too many exports' }] }),
			new ApiFailure(500, null),
			new TypeError('failed to fetch')
		]) {
			expect(failureOf(caught).signIn).toBe(false);
		}
	});

	it('asks for another try when the server faulted', () => {
		expect(failureMessage(new ApiFailure(500, null))).toBe(
			'We could not build your spreadsheet. Try again in a moment.'
		);
	});

	it('keeps the server words when the refusal actually carried some', () => {
		const body = { status: 429, errors: [{ message: 'too many exports' }] };
		expect(failureMessage(new ApiFailure(429, body))).toBe('too many exports');
	});

	// The console can be newer than the control plane it asks, so a 404 with no
	// body is a state a seller reaches rather than a hypothetical one.
	it('words a refusal that carried no body, rather than passing the transport string on', () => {
		for (const status of [403, 404, 429]) {
			expect(failureMessage(new ApiFailure(status, null))).toBe(REFUSED_WITHOUT_REASON);
		}
	});

	it('words a refusal whose body carried no message either', () => {
		expect(failureMessage(new ApiFailure(404, { status: 404, errors: [] }))).toBe(
			REFUSED_WITHOUT_REASON
		);
	});

	it('says the request never landed when nothing answered', () => {
		expect(failureMessage(new TypeError('failed to fetch'))).toBe(
			'We could not reach Teachouse. Check your internet connection and try again.'
		);
	});

	it('words every arm as a sentence with no status code in it', () => {
		const caughts = [
			new ApiFailure(401, null),
			new ApiFailure(403, null),
			new ApiFailure(404, null),
			new ApiFailure(429, null),
			new ApiFailure(500, null),
			new ApiFailure(503, null),
			new Error('x')
		];
		for (const caught of caughts) {
			const message = failureMessage(caught);
			expect(message).toMatch(/\.$/);
			expect(message).not.toMatch(/\b[45]\d\d\b/);
			expect(message).not.toMatch(/request failed/i);
		}
	});
});

describe('one press of the export button', () => {
	it('hands the document over and reports the name it used', async () => {
		const save = vi.fn();
		const state = await runExport(() => Promise.resolve(csvDocument()), save);
		expect(state).toEqual({ kind: 'handed', filename: SERVER_NAME });
		expect(save).toHaveBeenCalledTimes(1);
		expect(save.mock.calls[0]?.[0]).toMatchObject({ filename: SERVER_NAME });
	});

	// The page may not claim a save it cannot observe: a host that drops the
	// download is indistinguishable from one that kept it, so the terminal
	// state names the handover and never the file on disk.
	it('never reports that the file was saved, only that it was handed over', async () => {
		const state = await runExport(() => Promise.resolve(csvDocument()), vi.fn());
		expect(state.kind).toBe('handed');
		expect(JSON.stringify(state)).not.toMatch(/saved/i);
	});

	it('hands nothing over when the request failed', async () => {
		const save = vi.fn();
		const state = await runExport(() => Promise.reject(new ApiFailure(401, null)), save);
		expect(state).toEqual({ kind: 'failed', message: SESSION_ENDED, signIn: true });
		expect(save).not.toHaveBeenCalled();
	});

	it('separates a window that would not take the file from a request that never landed', async () => {
		const state = await runExport(
			() => Promise.resolve(csvDocument()),
			() => {
				throw new Error('this webview takes nothing');
			}
		);
		expect(state).toEqual({ kind: 'failed', message: HANDOFF_FAILED, signIn: false });
		expect(HANDOFF_FAILED).not.toBe(failureMessage(new Error('this webview takes nothing')));
	});
});
