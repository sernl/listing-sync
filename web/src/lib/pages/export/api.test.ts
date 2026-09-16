import { describe, expect, it } from 'vitest';
import { ApiFailure } from '$lib/api';
import { EXPORT_PATH, fetchCatalogueCsv } from './api';
import { FALLBACK_FILENAME } from './download';

const SERVER_NAME = 'teachouse-resources-2026-09-05.csv';
const CSV = 'Resource ID,Title\r\n1,Fractions\r\n';

function answering(response: Response): { fetch: typeof fetch; seen: Request[] } {
	const seen: Request[] = [];
	const impl = ((input: RequestInfo | URL, init?: RequestInit) => {
		seen.push(new Request(new URL(String(input), 'https://teachouse.io'), init));
		return Promise.resolve(response);
	}) as typeof fetch;
	return { fetch: impl, seen };
}

function csvResponse(disposition: string | null = `attachment; filename="${SERVER_NAME}"`) {
	const headers = new Headers({ 'content-type': 'text/csv; charset=utf-8' });
	if (disposition !== null) {
		headers.set('content-disposition', disposition);
	}
	return new Response(CSV, { status: 200, headers });
}

function errorResponse(status: number, body: unknown) {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json' }
	});
}

describe('asking the server for the catalogue', () => {
	it('gets the document and the name the server gave it', async () => {
		const { fetch: impl } = answering(csvResponse());
		const csv = await fetchCatalogueCsv(impl);
		expect(csv.filename).toBe(SERVER_NAME);
		expect(await csv.blob.text()).toBe(CSV);
	});

	it('asks the one export route, by relative path so the session cookie rides along', async () => {
		const { fetch: impl, seen } = answering(csvResponse());
		await fetchCatalogueCsv(impl);
		expect(EXPORT_PATH).toBe('/v1/products/export');
		expect(seen).toHaveLength(1);
		expect(new URL(seen[0]?.url ?? '').pathname).toBe(EXPORT_PATH);
		expect(seen[0]?.method).toBe('GET');
	});

	it('asks for a spreadsheet, so a shared error page is not read as one', async () => {
		const { fetch: impl, seen } = answering(csvResponse());
		await fetchCatalogueCsv(impl);
		expect(seen[0]?.headers.get('accept')).toBe('text/csv');
	});

	it('names the file even when the server sent no disposition', async () => {
		const { fetch: impl } = answering(csvResponse(null));
		expect((await fetchCatalogueCsv(impl)).filename).toBe(FALLBACK_FILENAME);
	});

	it('raises the shared failure, carrying the structured body, when refused', async () => {
		const body = {
			status: 401,
			errors: [{ code: 'session_required', kind: 'unauthenticated', message: 'a valid session is required' }]
		};
		const { fetch: impl } = answering(errorResponse(401, body));
		const caught = await fetchCatalogueCsv(impl).catch((error: unknown) => error);
		expect(caught).toBeInstanceOf(ApiFailure);
		expect((caught as ApiFailure).status).toBe(401);
		expect((caught as ApiFailure).code()).toBe('session_required');
	});

	it('still raises a failure when the refusal carried no readable body', async () => {
		const { fetch: impl } = answering(new Response('<html>502</html>', { status: 502 }));
		const caught = await fetchCatalogueCsv(impl).catch((error: unknown) => error);
		expect(caught).toBeInstanceOf(ApiFailure);
		expect((caught as ApiFailure).status).toBe(502);
	});

	it('never reads a failing answer as a spreadsheet', async () => {
		const { fetch: impl } = answering(errorResponse(500, { status: 500, errors: [] }));
		await expect(fetchCatalogueCsv(impl)).rejects.toBeInstanceOf(ApiFailure);
	});
});
