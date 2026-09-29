// The API's transport: one fetch wrapper and the structured error parsed into
// a typed failure. Apart from `$lib/api` so a screen that talks to the API
// through a call or two -- the sign-in screens, which must paint from as
// little script as possible -- does not also download the client for every
// endpoint the console consumes. `$lib/api` re-exports all of it, and is
// where every other caller reads it from.

import type { APIErrorCode, APIErrorKind } from '$lib/generated/vocab';

export interface APIErrorEntry {
	code?: APIErrorCode;
	kind?: APIErrorKind;
	message: string;
	detail?: unknown;
}

export interface APIErrorBody {
	status: number;
	errors: APIErrorEntry[];
}

export interface ApiResponseContext {
	requested_path: string;
	final_path: string | null;
	content_type: string | null;
	redirected: boolean;
	problem?: 'unexpected_content_type' | 'invalid_json' | 'invalid_error_body';
}

/** A failing response, thrown with its structured body when one existed. */
export class ApiFailure extends Error {
	readonly status: number;
	readonly body: APIErrorBody | null;

	constructor(
		status: number,
		body: APIErrorBody | null,
		readonly response: ApiResponseContext | null = null
	) {
		super(
			response?.problem
				? `Unexpected API response (${status}); reload or report this request.`
				: (body?.errors?.[0]?.message ?? `request failed with ${status}`)
		);
		this.status = status;
		this.body = body;
	}

	code(): APIErrorCode | undefined {
		return this.body?.errors?.[0]?.code;
	}
}

export async function request<T>(path: string, init?: RequestInit): Promise<T> {
	const response = await fetch(path, {
		...init,
		headers: { accept: 'application/json', ...(init?.headers ?? {}) }
	});
	if (response.status === 204) {
		return undefined as T;
	}
	const context: ApiResponseContext = {
		requested_path: path.split(/[?#]/, 1)[0],
		final_path: response.url ? new URL(response.url).pathname : null,
		content_type: response.headers.get('content-type'),
		redirected: response.redirected
	};
	const mediaType = context.content_type?.split(';', 1)[0].trim().toLowerCase();
	if (mediaType !== 'application/json' && !mediaType?.endsWith('+json')) {
		throw new ApiFailure(response.status, null, {
			...context,
			problem: 'unexpected_content_type'
		});
	}
	let body: unknown;
	try {
		body = await response.json();
	} catch {
		throw new ApiFailure(response.status, null, {
			...context,
			problem: 'invalid_json'
		});
	}
	if (!response.ok) {
		const envelope =
			typeof body === 'object' &&
			body !== null &&
			'errors' in body &&
			Array.isArray(body.errors) &&
			body.errors.every(
				(entry: unknown) =>
					typeof entry === 'object' &&
					entry !== null &&
					'message' in entry &&
					typeof entry.message === 'string'
			) &&
			'status' in body &&
			typeof body.status === 'number';
		throw envelope
			? new ApiFailure(response.status, body as APIErrorBody, context)
			: new ApiFailure(response.status, null, {
					...context,
					problem: 'invalid_error_body'
				});
	}
	return body as T;
}

export function post<T>(path: string, body: unknown, headers?: Record<string, string>): Promise<T> {
	return request<T>(path, {
		method: 'POST',
		headers: { 'content-type': 'application/json', ...(headers ?? {}) },
		body: JSON.stringify(body)
	});
}

export function put<T>(path: string, body: unknown): Promise<T> {
	return request<T>(path, {
		method: 'PUT',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify(body)
	});
}

export function patch<T>(path: string, body: unknown): Promise<T> {
	return request<T>(path, {
		method: 'PATCH',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify(body)
	});
}
