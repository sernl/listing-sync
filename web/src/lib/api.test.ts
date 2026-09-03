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

/** The slice of XMLHttpRequest the upload drives, scripted so the progress
 *  events and the response are this test's to choose. */
class FakeUpload {
	static last: FakeUpload | null = null;
	static reply: { status: number; body: unknown } = { status: 201, body: {} };

	method = '';
	url = '';
	headers: Record<string, string> = {};
	sentBody: unknown = null;
	status = 0;
	responseText = '';
	upload = { listeners: new Map<string, (event: unknown) => void>(), addEventListener(kind: string, run: (event: unknown) => void) { this.listeners.set(kind, run); } };
	private listeners = new Map<string, () => void>();

	constructor() {
		FakeUpload.last = this;
	}

	open(method: string, url: string) {
		this.method = method;
		this.url = url;
	}

	setRequestHeader(name: string, value: string) {
		this.headers[name] = value;
	}

	addEventListener(kind: string, run: () => void) {
		this.listeners.set(kind, run);
	}

	send(body: unknown) {
		this.sentBody = body;
		this.upload.listeners.get('progress')?.({ lengthComputable: true, loaded: 5, total: 10 });
		this.status = FakeUpload.reply.status;
		this.responseText = JSON.stringify(FakeUpload.reply.body);
		this.listeners.get('load')?.();
	}
}

describe('the upload', () => {
	it('sends the raw file to the archive mode it was asked for, reporting progress', async () => {
		FakeUpload.reply = {
			status: 201,
			body: { payload: [], cover: {}, previews: [], stored_bytes: 1, storage_bytes_max: 2 }
		};
		vi.stubGlobal('XMLHttpRequest', FakeUpload);
		const file = new File(['bytes'], 'pack.zip');
		const seen: number[] = [];
		const landed = await api.upload(file, 'keep_whole', (fraction) => seen.push(fraction));
		expect(FakeUpload.last?.method).toBe('POST');
		expect(FakeUpload.last?.url).toBe('/v1/uploads?archive=keep_whole');
		expect(FakeUpload.last?.sentBody).toBe(file);
		expect(seen).toEqual([0.5]);
		expect(landed.storage_bytes_max).toBe(2);
	});

	it('throws the structured refusal, so a quota answer keeps its detail', async () => {
		FakeUpload.reply = {
			status: 422,
			body: {
				status: 422,
				errors: [
					{
						code: 'quota_exceeded',
						kind: 'validation',
						message: "this plan's storage quota is full",
						detail: { quota: 'storage_bytes_max', used: 9, limit: 10 }
					}
				]
			}
		};
		vi.stubGlobal('XMLHttpRequest', FakeUpload);
		const failure = await api
			.upload(new File(['b'], 'a.pdf'), 'explode')
			.catch((caught: unknown) => caught);
		expect(failure).toBeInstanceOf(ApiFailure);
		expect((failure as ApiFailure).code()).toBe('quota_exceeded');
		expect((failure as ApiFailure).body?.errors[0].detail).toEqual({
			quota: 'storage_bytes_max',
			used: 9,
			limit: 10
		});
	});
});

describe('the authoring endpoints', () => {
	it('states the publish intent on the job it enqueues', async () => {
		const seen: Array<{ url: string; body: unknown }> = [];
		vi.stubGlobal(
			'fetch',
			vi.fn(async (url: string, init?: RequestInit) => {
				seen.push({ url, body: JSON.parse(String(init?.body)) });
				return jsonResponse(201, { job: 'j', replay: false });
			})
		);
		await api.createJob('Tpt', ['m1'], 'key-1', 'live');
		expect(seen[0].body).toEqual({ inventory: 'Tpt', mappings: ['m1'], intent: 'live' });
	});

	it('sends the delete election as a body on the DELETE itself', async () => {
		const seen: Array<{ url: string; method?: string; body: unknown }> = [];
		vi.stubGlobal(
			'fetch',
			vi.fn(async (url: string, init?: RequestInit) => {
				seen.push({ url, method: init?.method, body: JSON.parse(String(init?.body)) });
				return jsonResponse(200, { product: 'p', removals: [], left_live: [] });
			})
		);
		await api.deleteProduct('p1', { remove_from: ['TesGb'], leave_live: false });
		expect(seen[0].method).toBe('DELETE');
		expect(seen[0].url).toBe('/v1/products/p1');
		expect(seen[0].body).toEqual({ remove_from: ['TesGb'], leave_live: false });
	});

	it('adds a marketplace to an item under that item', async () => {
		const seen: Array<{ url: string; method?: string; body: unknown }> = [];
		vi.stubGlobal(
			'fetch',
			vi.fn(async (url: string, init?: RequestInit) => {
				seen.push({ url, method: init?.method, body: JSON.parse(String(init?.body)) });
				return jsonResponse(201, {
					id: 'm-new',
					product: 'p1',
					inventory: 'Tpt',
					binding_state: 'unbound',
					lifecycle_state: 'absent',
					updated_at: 5000
				});
			})
		);
		const head = await api.addMapping('p1', 'Tpt');
		expect(seen[0].method).toBe('POST');
		expect(seen[0].url).toBe('/v1/products/p1/mappings');
		expect(seen[0].body).toEqual({ inventory: 'Tpt' });
		expect(head.id).toBe('m-new');
		expect(head.binding_state).toBe('unbound');
	});

	it('binds a listing under the mapping, sending the URL the seller pasted', async () => {
		const seen: Array<{ url: string; method?: string; body: unknown }> = [];
		vi.stubGlobal(
			'fetch',
			vi.fn(async (url: string, init?: RequestInit) => {
				seen.push({ url, method: init?.method, body: JSON.parse(String(init?.body)) });
				return jsonResponse(200, {
					id: 'm1',
					product: 'p1',
					inventory: 'Tpt',
					binding_state: 'bound',
					lifecycle_state: 'live',
					updated_at: 5000,
					listing_url: 'https://www.teacherspayteachers.com/Product/listing-17511712'
				});
			})
		);
		const head = await api.bindMapping(
			'm1',
			'https://www.teacherspayteachers.com/Product/fractions-17511712'
		);
		expect(seen[0].method).toBe('POST');
		expect(seen[0].url).toBe('/v1/mappings/m1/bind');
		expect(seen[0].body).toEqual({
			listing_url: 'https://www.teacherspayteachers.com/Product/fractions-17511712'
		});
		expect(head.binding_state).toBe('bound');
	});

	it('surfaces the bind refusal with its code, so a bad link reads as a bad link', async () => {
		vi.stubGlobal(
			'fetch',
			vi.fn(async () =>
				jsonResponse(422, {
					status: 422,
					errors: [
						{
							code: 'listing_url_unusable',
							kind: 'validation',
							message: 'that link is a listing on a different marketplace'
						}
					]
				})
			)
		);
		const failure = await api.bindMapping('m1', 'https://www.tes.com/teaching-resource/x-1').catch(
			(caught: unknown) => caught
		);
		expect((failure as ApiFailure).code()).toBe('listing_url_unusable');
	});

	it('narrows the catalogue by label without disturbing the cursor walk', async () => {
		const seen: string[] = [];
		vi.stubGlobal(
			'fetch',
			vi.fn(async (url: string) => {
				seen.push(url);
				return jsonResponse(200, { products: [], next_cursor: null });
			})
		);
		await api.products(null, 'Autumn term');
		await api.products('cur-1', 'Autumn term');
		await api.products();
		expect(seen).toEqual([
			'/v1/products?label=Autumn+term',
			'/v1/products?cursor=cur-1&label=Autumn+term',
			'/v1/products'
		]);
	});

	it('replaces an item\'s labels with the whole set, not a delta', async () => {
		const seen: Array<{ url: string; method?: string; body: unknown }> = [];
		vi.stubGlobal(
			'fetch',
			vi.fn(async (url: string, init?: RequestInit) => {
				seen.push({ url, method: init?.method, body: JSON.parse(String(init?.body)) });
				return jsonResponse(200, { labels: [{ name: 'Bundle', colour: 'teal' }] });
			})
		);
		const view = await api.setProductLabels('p1', ['Bundle']);
		expect(seen[0].method).toBe('PUT');
		expect(seen[0].url).toBe('/v1/products/p1/labels');
		expect(seen[0].body).toEqual({ labels: ['Bundle'] });
		expect(view.labels[0].colour).toBe('teal');
	});

	it('reads one marketplace vocabulary per inventory', async () => {
		const seen: string[] = [];
		vi.stubGlobal(
			'fetch',
			vi.fn(async (url: string) => {
				seen.push(url);
				return jsonResponse(200, { inventory: 'TesGb' });
			})
		);
		await api.vocabulary('TesGb');
		expect(seen).toEqual(['/v1/vocabulary/TesGb']);
	});
});
