// What this module puts on the wire, against a stubbed `fetch`.
//
// It exists because the shape of these five calls is the one thing in the
// slice that type-checking cannot verify: `call` casts a JSON body to a
// declared type, so a client type that disagrees with the Rust it names
// compiles cleanly and fails only in a browser. The list-versus-detail split
// below is the specific disagreement that broke this tab once — the list
// serves heads without drafts, and a client typed as though it did threw on
// the first saved template.

import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiFailure } from '$lib/api';
import { templates } from './api';

interface Call {
	url: string;
	method: string;
	headers: Record<string, string>;
	body: unknown;
}

const calls: Call[] = [];

function answering(status: number, body: unknown) {
	vi.stubGlobal('fetch', (url: string, init?: RequestInit) => {
		calls.push({
			url,
			method: init?.method ?? 'GET',
			headers: (init?.headers ?? {}) as Record<string, string>,
			body: init?.body === undefined ? undefined : JSON.parse(String(init.body))
		});
		return Promise.resolve({
			ok: status >= 200 && status < 300,
			status,
			json: () => Promise.resolve(body)
		} as Response);
	});
}

afterEach(() => {
	calls.length = 0;
	vi.unstubAllGlobals();
});

describe('reading templates', () => {
	it('lists from the collection and unwraps the envelope', async () => {
		const rows = [{ id: 'a', name: 'One', created_at: 1, updated_at: 2 }];
		answering(200, { templates: rows });
		expect(await templates.list()).toEqual(rows);
		expect(calls[0].url).toBe('/v1/templates');
		expect(calls[0].method).toBe('GET');
	});

	it('reads one template from the item route, which is where a draft lives', async () => {
		answering(200, { id: 'a', name: 'One', created_at: 1, updated_at: 2, draft: { free: true } });
		const one = await templates.read('a');
		expect(one.draft).toEqual({ free: true });
		expect(calls[0].url).toBe('/v1/templates/a');
	});

	it('escapes the identifier into the path', async () => {
		answering(200, {});
		await templates.read('a/../b');
		expect(calls[0].url).toBe('/v1/templates/a%2F..%2Fb');
	});
});

describe('writing templates', () => {
	const input = { name: 'One', draft: { free: true } };

	it('creates with POST, JSON content type, and the draft wrapped', async () => {
		answering(201, { id: 'a', name: 'One', created_at: 1, updated_at: 1, draft: {} });
		await templates.create(input);
		expect(calls[0]).toMatchObject({
			url: '/v1/templates',
			method: 'POST',
			headers: { 'content-type': 'application/json' },
			body: { name: 'One', draft: { free: true } }
		});
	});

	it('edits with PATCH rather than PUT, which is not mounted', async () => {
		answering(200, { id: 'a', name: 'One', created_at: 1, updated_at: 2, draft: {} });
		await templates.update('a', input);
		expect(calls[0].method).toBe('PATCH');
		expect(calls[0].url).toBe('/v1/templates/a');
	});

	it('removes with DELETE and accepts the empty 204 body', async () => {
		answering(204, undefined);
		await expect(templates.remove('a')).resolves.toBeUndefined();
		expect(calls[0].method).toBe('DELETE');
	});
});

describe('a refusal', () => {
	it('arrives as an ApiFailure carrying the server’s own sentence', async () => {
		answering(422, {
			status: 422,
			errors: [{ message: 'a template of that name already exists', kind: 'validation' }]
		});
		await expect(templates.create({ name: 'One', draft: {} })).rejects.toThrow(
			'a template of that name already exists'
		);
	});

	it('carries the status, so a caller can tell a missing template from a refusal', async () => {
		answering(404, { status: 404, errors: [{ message: 'no such template' }] });
		const failure = await templates.read('a').catch((error: unknown) => error);
		expect(failure).toBeInstanceOf(ApiFailure);
		expect((failure as ApiFailure).status).toBe(404);
	});
});
