import { afterEach, describe, expect, it, vi } from 'vitest';
import { allPages, api, ApiFailure } from './api';

afterEach(() => {
	vi.unstubAllGlobals();
});

function jsonResponse(status: number, body: unknown): Response {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json' }
	});
}

describe('the api client', () => {
	it('parses the structured error into a typed failure', async () => {
		vi.stubGlobal(
			'fetch',
			vi.fn(async () =>
				jsonResponse(401, {
					status: 401,
					errors: [{ code: 'session_required', kind: 'unauthenticated', message: 'no' }]
				})
			)
		);
		const failure = await api.whoami().catch((caught: unknown) => caught);
		expect(failure).toBeInstanceOf(ApiFailure);
		expect((failure as ApiFailure).status).toBe(401);
		expect((failure as ApiFailure).code()).toBe('session_required');
	});

	it('a body-less failure still throws with its status', async () => {
		vi.stubGlobal('fetch', vi.fn(async () => new Response('', { status: 502 })));
		const failure = await api.whoami().catch((caught: unknown) => caught);
		expect((failure as ApiFailure).status).toBe(502);
		expect((failure as ApiFailure).body).toBeNull();
	});

	it('the idempotency key travels on the sync-starting request', async () => {
		const seen: Array<{ url: string; headers: Headers }> = [];
		vi.stubGlobal(
			'fetch',
			vi.fn(async (url: string, init?: RequestInit) => {
				seen.push({ url, headers: new Headers(init?.headers) });
				return jsonResponse(201, { job: 'j', replay: false });
			})
		);
		await api.createJob('TesNz', ['m1'], 'key-123');
		expect(seen[0].url).toBe('/v1/jobs');
		expect(seen[0].headers.get('idempotency-key')).toBe('key-123');
	});

	it('allPages walks cursors until the server stops minting them', async () => {
		const pages = [
			{ rows: [1, 2], next_cursor: 'a' },
			{ rows: [3], next_cursor: null }
		];
		let calls = 0;
		const collected = await allPages(
			async (cursor) => {
				expect(cursor ?? null).toBe(calls === 0 ? null : 'a');
				calls += 1;
				return pages[calls - 1];
			},
			(page) => page.rows
		);
		expect(collected).toEqual([1, 2, 3]);
	});
});
