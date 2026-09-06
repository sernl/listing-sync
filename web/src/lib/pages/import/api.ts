// The two spreadsheet-import calls the shared client cannot make: every
// function in `$lib/api` sends and reads JSON, and these two send raw bytes
// and read a workbook.
//
// The failure path is the shared one on purpose. Both routes answer a non-2xx
// with the same structured error body as every other route, so both are parsed
// into the same `ApiFailure` the rest of the console already branches on, and
// this page gains no second error vocabulary.

import { ApiFailure, type APIErrorBody, type UploadedBatchView } from '$lib/api';

/** A document as it came off the wire, already named. */
export interface NamedDocument {
	blob: Blob;
	filename: string;
}

/** The generated template workbook. Same origin and same session cookie as
 *  every other call; the route takes no parameters. */
export const TEMPLATE_PATH = '/v1/imports/template';

/** What the workbook is called when the server did not say.
 *
 * The server's own name, which is undated on purpose: a template is a blank
 * form, and a second download of a form replaces the first in a downloads
 * folder rather than accumulating beside it. */
export const TEMPLATE_FILENAME = 'teachouse-import-template.xlsx';

/** The name the server gave this document, read out of `Content-Disposition`.
 *
 * Its own rather than the export page's parser, which hard-codes the export's
 * fallback name and takes no parameter for another; sharing it would mean
 * editing that module for this one's benefit. Readable at all only because the
 * console and the API are one origin, so no header is hidden behind CORS
 * exposure rules.
 *
 * A name carrying a path separator falls back rather than being cleaned: every
 * name this route sends is the constant above, so a separator would mean
 * something upstream is no longer what this parser was written against. */
export function templateNameFrom(disposition: string | null): string {
	if (disposition === null) {
		return TEMPLATE_FILENAME;
	}
	const match = /(?:^|;)\s*filename\s*=\s*(?:"([^"]*)"|([^;]*))/i.exec(disposition);
	const raw = (match?.[1] ?? match?.[2] ?? '').trim();
	if (raw === '' || raw.includes('/') || raw.includes('\\')) {
		return TEMPLATE_FILENAME;
	}
	return raw;
}

async function refusalFrom(response: Response): Promise<ApiFailure> {
	let body: APIErrorBody | null = null;
	try {
		body = (await response.json()) as APIErrorBody;
	} catch {
		body = null;
	}
	return new ApiFailure(response.status, body);
}

/** Fetch the blank template workbook, named the way the server named it.
 *
 * `fetchImpl` is an argument so this tests without a network. */
export async function fetchTemplate(fetchImpl: typeof fetch = fetch): Promise<NamedDocument> {
	const response = await fetchImpl(TEMPLATE_PATH, {
		headers: {
			accept: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'
		}
	});
	if (!response.ok) {
		throw await refusalFrom(response);
	}
	return {
		blob: await response.blob(),
		filename: templateNameFrom(response.headers.get('content-disposition'))
	};
}

/** Post a filled sheet and read back the batch it parsed to.
 *
 * The filename travels as a query parameter rather than a header because the
 * body is bytes and a `.csv` has no other way to say which tab it is; the
 * server's own `UploadParams` documents that. The key is the batch's identity
 * on the server rather than a deduplication token beside it, so a retried
 * submit is the same batch and `created` is how a client tells the retry from
 * the first delivery. */
export async function uploadSheet(
	file: File,
	idempotencyKey: string,
	fetchImpl: typeof fetch = fetch
): Promise<UploadedBatchView> {
	const response = await fetchImpl(`/v1/imports?name=${encodeURIComponent(file.name)}`, {
		method: 'POST',
		headers: { accept: 'application/json', 'idempotency-key': idempotencyKey },
		body: file
	});
	if (!response.ok) {
		throw await refusalFrom(response);
	}
	return (await response.json()) as UploadedBatchView;
}

/** The batch a refused upload says is already open, where it said so.
 *
 * Read from the refusal's own detail rather than re-listing the imports: the
 * server names it in the body it refuses with, and a second read could answer
 * a different batch than the one that caused the refusal. */
export function openBatchFrom(caught: unknown): string | null {
	if (!(caught instanceof ApiFailure) || caught.status !== 409) {
		return null;
	}
	const detail = caught.body?.errors?.[0]?.detail;
	if (typeof detail !== 'object' || detail === null) {
		return null;
	}
	const open = (detail as Record<string, unknown>).open_batch;
	return typeof open === 'string' ? open : null;
}
