import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiFailure } from '$lib/api';
import { countCarriers, deleteLabel, renameLabel } from './api';

/** One recorded call, and the answer the stub gave it. */
interface Call {
	url: string;
	init: RequestInit;
}

function stubFetch(answers: { status: number; body?: unknown }[]): Call[] {
	const calls: Call[] = [];
	let at = 0;
	vi.stubGlobal('fetch', (url: string, init: RequestInit = {}) => {
		calls.push({ url, init });
		const answer = answers[Math.min(at, answers.length - 1)];
		at += 1;
		return Promise.resolve({
			status: answer.status,
			ok: answer.status >= 200 && answer.status < 300,
			json: () => Promise.resolve(answer.body)
		} as Response);
	});
	return calls;
}

function headersOf(init: RequestInit): Record<string, string> {
	return (init.headers ?? {}) as Record<string, string>;
}

afterEach(() => {
	vi.unstubAllGlobals();
});

describe('renaming a label', () => {
	it('patches the label by its own name and answers the label as stored', async () => {
		const calls = stubFetch([{ status: 200, body: { name: 'Spring term', colour: 'green' } }]);
		const stored = await renameLabel('Autumn term', 'Spring term');
		expect(stored).toEqual({ name: 'Spring term', colour: 'green' });
		expect(calls).toHaveLength(1);
		expect(calls[0].url).toBe('/v1/labels/Autumn%20term');
		expect(calls[0].init.method).toBe('PATCH');
		expect(calls[0].init.body).toBe(JSON.stringify({ name: 'Spring term' }));
	});

	// The name is one path segment, so anything a path reserves has to travel
	// escaped or the request reaches a route that does not exist.
	it('escapes everything a path reserves', async () => {
		const calls = stubFetch([{ status: 200, body: { name: 'x', colour: 'slate' } }]);
		await renameLabel('Year 5 & 6 · #1', 'x');
		expect(calls[0].url).toBe('/v1/labels/Year%205%20%26%206%20%C2%B7%20%231');
	});

	// The header merge this slice deliberately corrected: a caller's own headers
	// must add to the accept rather than replace it.
	it('sends both the accept and the content type', async () => {
		const calls = stubFetch([{ status: 200, body: { name: 'x', colour: 'slate' } }]);
		await renameLabel('a', 'x');
		expect(headersOf(calls[0].init)).toEqual({
			accept: 'application/json',
			'content-type': 'application/json'
		});
	});

	it('raises the refusal the server wrote, so the field can answer with it', async () => {
		stubFetch([
			{
				status: 422,
				body: {
					status: 422,
					errors: [{ message: 'that name is already one of your labels' }]
				}
			}
		]);
		await expect(renameLabel('Bundles', 'Phonics')).rejects.toBeInstanceOf(ApiFailure);
	});
});

describe('deleting a label', () => {
	it('deletes by name and answers nothing, because 204 carries no body', async () => {
		const calls = stubFetch([{ status: 204 }]);
		await expect(deleteLabel('Autumn term')).resolves.toBeUndefined();
		expect(calls[0].init.method).toBe('DELETE');
		expect(calls[0].url).toBe('/v1/labels/Autumn%20term');
	});
});

describe('counting the carriers of one label', () => {
	it('asks for the largest page and narrows it to the label', async () => {
		const calls = stubFetch([{ status: 200, body: { products: [], next_cursor: null } }]);
		expect(await countCarriers('Bundles')).toBe(0);
		expect(calls[0].url).toBe('/v1/products?label=Bundles&limit=200');
	});

	it('stops on a page that carries no cursor', async () => {
		const calls = stubFetch([
			{ status: 200, body: { products: [{}, {}, {}], next_cursor: null } }
		]);
		expect(await countCarriers('Bundles')).toBe(3);
		expect(calls).toHaveLength(1);
	});

	it('hands the cursor back and sums the pages', async () => {
		const calls = stubFetch([
			{ status: 200, body: { products: Array(200).fill({}), next_cursor: 'c1' } },
			{ status: 200, body: { products: Array(200).fill({}), next_cursor: 'c2' } },
			{ status: 200, body: { products: Array(7).fill({}), next_cursor: null } }
		]);
		expect(await countCarriers('Bundles')).toBe(407);
		expect(calls).toHaveLength(3);
		expect(calls[1].url).toBe('/v1/products?label=Bundles&limit=200&cursor=c1');
		expect(calls[2].url).toBe('/v1/products?label=Bundles&limit=200&cursor=c2');
	});

	// A full page with a cursor is asked again even though the next page turns
	// out to be empty: the server mints a cursor whenever the page filled, so
	// stopping on a full page would drop every carrier past the first two
	// hundred.
	it('asks once more after a page that exactly filled', async () => {
		const calls = stubFetch([
			{ status: 200, body: { products: Array(200).fill({}), next_cursor: 'c1' } },
			{ status: 200, body: { products: [], next_cursor: null } }
		]);
		expect(await countCarriers('Bundles')).toBe(200);
		expect(calls).toHaveLength(2);
	});

	it('treats an empty cursor as the end rather than asking for it', async () => {
		const calls = stubFetch([
			{ status: 200, body: { products: [{}], next_cursor: '' } },
			{ status: 200, body: { products: [{}], next_cursor: null } }
		]);
		expect(await countCarriers('Bundles')).toBe(1);
		expect(calls).toHaveLength(1);
	});
});

describe('creating a label on resources', () => {
	it('adds the name to each resource and keeps the labels it already carries, minus system ones', async () => {
		const { labelResources } = await import('./api');
		const written: [string, string[]][] = [];
		const held: Record<string, { name: string; colour: string; system: boolean }[]> = {
			a: [
				{ name: 'Maths', colour: 'blue', system: false },
				{ name: 'From TPT', colour: 'green', system: true }
			],
			b: [{ name: 'autumn term', colour: 'red', system: false }]
		};
		const done = await labelResources(['a', 'b'], 'Autumn term', {
			read: (product) => Promise.resolve({ labels: held[product] }),
			write: (product, labels) => {
				written.push([product, labels]);
				return Promise.resolve();
			}
		});
		expect(done).toEqual({ count: 2, refusal: null });
		expect(written).toEqual([
			['a', ['Maths', 'Autumn term']],
			['b', ['autumn term']]
		]);
	});

	it('stops at the first refusal and says how many were labelled before it', async () => {
		const { labelResources } = await import('./api');
		let at = 0;
		const done = await labelResources(['a', 'b', 'c'], 'Bundle', {
			read: () => Promise.resolve({ labels: [] }),
			write: () => {
				at += 1;
				return at === 2
					? Promise.reject(
							new ApiFailure(422, { errors: [{ message: 'Twenty is the most.' }] } as never)
						)
					: Promise.resolve();
			}
		});
		expect(done.count).toBe(1);
		expect(done.refusal).toBe('Twenty is the most.');
	});
});
