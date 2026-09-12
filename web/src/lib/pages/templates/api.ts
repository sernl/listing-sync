// The New resource tab's own routes, beside the page rather than in
// `$lib/api`: a template is a named partial `DraftInput`, and these seven
// calls are what `crates/tam-api/src/resource_templates.rs` serves. Every
// call goes through `call`, so moving the collection is one edit to `BASE`.

import { ApiFailure, type APIErrorBody, type DraftInput } from '$lib/api';
import type { InventoryId, TemplateApplyVerdict } from '$lib/generated/vocab';

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
	/** What the seller wrote about when to reach for this template, or null.
	 *  On the head as well as the view, because the list is where the choice
	 *  between two templates is made. */
	description: string | null;
	/** The marketplace this template is written for, or null for one that
	 *  suits any of them. An `InventoryId` on the wire, as everywhere else;
	 *  the lowercase token is the column's own. */
	scope: InventoryId | null;
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
	description: string | null;
	scope: InventoryId | null;
	draft: DraftInput;
}

/** One resource the apply would touch, as the plan and the submit both decide
 *  it. The same shape the migration plan answers with, for the same reason: a
 *  seller ticking forty resources is told, per resource, what will happen
 *  before any of it does.
 *
 *  `fields` names what the apply would write, and is empty on anything but
 *  `will_change`. `reason` carries the server's own sentence where a resource
 *  is refused — a live listing whose edit we cannot make is the case — and is
 *  null otherwise. */
export interface TemplateApplyRow {
	product: string;
	title: string;
	verdict: TemplateApplyVerdict;
	fields: string[];
	reason: string | null;
}

export interface TemplateApplyCounts {
	will_change: number;
	unchanged: number;
	blocked: number;
}

export interface TemplateApplyPlanView {
	rows: TemplateApplyRow[];
	counts: TemplateApplyCounts;
}

/** What the submit answers: how many resources it wrote, left alone and
 *  refused. Rows are not repeated — the plan the seller confirmed already
 *  named them, and the submit re-runs it rather than trusting that copy. */
export interface TemplateApplyAck {
	changed: number;
	unchanged: number;
	blocked: number;
}

/** Which resources an apply is over: a selection the seller ticked, or a
 *  collection by identity. One of the two, never both. */
export type ApplySelection = { products: string[] } | { collection: string };

export interface TemplateApplyBody {
	selection: ApplySelection;
	/** Whether a field the resource already answers is written over. Off is
	 *  fill-the-empty-ones, which is what a template is for. */
	overwrite: boolean;
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
	remove: (id: string) => call<void>(at(id), { method: 'DELETE' }),
	/** What applying this template to a selection would do. Writes nothing and
	 *  takes no key, so it is asked again on every change of selection or of
	 *  the overwrite tick. */
	applyPlan: (id: string, body: TemplateApplyBody) =>
		call<TemplateApplyPlanView>(`${at(id)}/apply/plan`, {
			method: 'POST',
			headers: { 'content-type': 'application/json' },
			body: JSON.stringify(body)
		}),
	/** Confirm the plan. The key is the request's own identity, as it is for
	 *  a migration: a retry after a dropped answer replays the same apply
	 *  rather than writing the seller's catalogue twice. */
	apply: (id: string, body: TemplateApplyBody, idempotencyKey: string) =>
		call<TemplateApplyAck>(at(id) + '/apply', {
			method: 'POST',
			headers: { 'content-type': 'application/json', 'idempotency-key': idempotencyKey },
			body: JSON.stringify(body)
		})
};

/** The cache keys, held here rather than in `$lib/query` for the same reason
 *  the routes are: both retire together when a template becomes something
 *  `$lib/api` serves beside everything else. One draft is its own entry, so
 *  opening a template twice reads it once. */
export const templateKeys = {
	all: ['templates'] as const,
	one: (id: string) => ['templates', id] as const
};
