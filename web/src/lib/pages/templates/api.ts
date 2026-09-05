// The New resource tab's own routes, beside the page rather than in
// `$lib/api`: a template is a named partial `DraftInput`, and these five
// calls are what `crates/tam-api/src/resource_templates.rs` serves. Every
// call goes through `call`, so moving the collection is one edit to `BASE`.

import { ApiFailure, type APIErrorBody, type DraftInput } from '$lib/api';

const BASE = '/v1/templates';

/** One template as the list serves it: everything but the draft.
 *
 *  The draft is absent rather than empty, and the distinction is the reason
 *  this type exists separately from `TemplateView`. The listing is unpaged, so
 *  a hundred drafts at the server's byte ceiling would be several megabytes on
 *  every open; `ResourceTemplateHead` withholds them deliberately
 *  (`resource_templates.rs:116-127`). Anything needing a draft reads one
 *  through `read`. */
export interface TemplateHead {
	id: string;
	name: string;
	created_at: number;
	updated_at: number;
}

/** One template with its draft, as `GET /v1/templates/{id}` serves it.
 *
 *  `draft` is a partial of the same shape `/v1/authoring/check` validates, so
 *  a template is a stored partial of what the create form submits rather than
 *  a second vocabulary. Both instants are milliseconds since the epoch, which
 *  is what `tam_types::Timestamp` holds. */
export interface TemplateView extends TemplateHead {
	draft: DraftInput;
}

export interface TemplatesView {
	templates: TemplateHead[];
}

/** What a create or an edit names. The organisation is absent on purpose: the
 *  server takes it from the session, as every other write here does. The edit
 *  is a PATCH whose two fields are each optional and never both absent; this
 *  form always carries both, so one type serves each. */
export interface TemplateInput {
	name: string;
	draft: DraftInput;
}

async function call<T>(path: string, init?: RequestInit): Promise<T> {
	const response = await fetch(path, {
		headers: { accept: 'application/json', ...(init?.headers ?? {}) },
		...init
	});
	if (response.status === 204) {
		return undefined as T;
	}
	if (!response.ok) {
		let detail: APIErrorBody | null = null;
		try {
			detail = (await response.json()) as APIErrorBody;
		} catch {
			detail = null;
		}
		throw new ApiFailure(response.status, detail);
	}
	return (await response.json()) as T;
}

function sending(input: TemplateInput, method: 'POST' | 'PATCH'): RequestInit {
	return {
		method,
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify(input)
	};
}

function at(id: string): string {
	return `${BASE}/${encodeURIComponent(id)}`;
}

export const templates = {
	list: () => call<TemplatesView>(BASE).then((view) => view.templates),
	read: (id: string) => call<TemplateView>(at(id)),
	create: (input: TemplateInput) => call<TemplateView>(BASE, sending(input, 'POST')),
	update: (id: string, input: TemplateInput) =>
		call<TemplateView>(at(id), sending(input, 'PATCH')),
	remove: (id: string) => call<void>(at(id), { method: 'DELETE' })
};

/** The cache keys, held here rather than in `$lib/query` for the same reason
 *  the routes are: both retire together when a template becomes something
 *  `$lib/api` serves beside everything else. One draft is its own entry, so
 *  opening a template twice reads it once. */
export const templateKeys = {
	all: ['templates'] as const,
	one: (id: string) => ['templates', id] as const
};
