// The browser's handle on the compiled core.
//
// `crates/tam-core-wasm` exports the same functions `POST /v1/authoring/check`
// answers with, so the message a seller reads as they type is the server's own
// answer rather than a client's restatement of it (D28). Everything crossing
// the boundary is a JSON string, and it is the same JSON the HTTP endpoint
// carries, so the two sides read one wire format rather than two.
//
// The generated bindings are gitignored and produced by `just web-wasm`, which
// `web-check` and `web-dev` both run first.

import type { CheckView, DraftInput, FormCaps } from '$lib/api';
import type { InventoryId } from '$lib/generated/vocab';
import init, {
	check_draft,
	core_version,
	initSync,
	project_preview,
	selection_caps
} from './generated/core.js';
import wasmUrl from './generated/core_bg.wasm?url';

/** A boundary failure, which is never a verdict: a verdict with no refusals
 *  means the draft is submittable, so an error rendered as one would read as
 *  approval. */
export class CoreFailure extends Error {
	readonly kind: string;

	constructor(kind: string, message: string) {
		super(message);
		this.name = 'CoreFailure';
		this.kind = kind;
	}
}

interface FailureShape {
	error: { kind: string; message: string };
}

function decoded<T>(json: string): T {
	const parsed: unknown = JSON.parse(json);
	if (typeof parsed === 'object' && parsed !== null && 'error' in parsed) {
		const { error } = parsed as FailureShape;
		throw new CoreFailure(error.kind, error.message);
	}
	return parsed as T;
}

/** What one marketplace carries for one canonical field, and what it drops.
 *
 *  `cap` absent is unmeasured, never unlimited, and `loss` null is "nothing
 *  dropped" rather than "nothing recorded to drop". */
export interface CoreProjectedRow {
	key: string;
	label: string;
	value: string;
	cap: { limit: number; unit: string } | null;
	required: boolean;
	loss: string | null;
}

/** The field half of a marketplace's projection.
 *
 *  Field rows only. `undecided_axes` names the equivalence axes this preview
 *  does not decide, because the relation behind them lives in Postgres and a
 *  mapping invented in the browser would be one nobody recorded. Those rows
 *  arrive from the server when the projection endpoint lands. */
export interface CoreProjection {
	inventory: InventoryId;
	rows: CoreProjectedRow[];
	undecided_axes: string[];
}

export interface Core {
	/** What the form refuses, decided by the function the server calls.
	 *
	 *  `caps` is omitted on every production path, which reads the capture
	 *  compiled into the module and so cannot disagree with the server. */
	checkDraft(draft: DraftInput, caps?: FormCaps | null): CheckView;
	/** What one marketplace will carry, from the compiled-in field registry. */
	projectPreview(draft: DraftInput, marketplace: InventoryId): CoreProjection;
	/** The measured picker caps, read from the same capture the server reads. */
	selectionCaps(): FormCaps;
	version(): string;
}

const bound: Core = {
	checkDraft: (draft, caps) =>
		decoded<CheckView>(
			check_draft(JSON.stringify(draft), caps === undefined || caps === null ? undefined : JSON.stringify(caps))
		),
	projectPreview: (draft, marketplace) =>
		decoded<CoreProjection>(project_preview(JSON.stringify(draft), marketplace)),
	selectionCaps: () => decoded<FormCaps>(selection_caps()),
	version: () => core_version()
};

let loaded: Core | null = null;
let loading: Promise<Core> | null = null;

/** The module once it is ready, or null while it is still arriving.
 *
 *  Synchronous by design: the create form's refusals are computed inside a
 *  `$derived`, and a rule that had to be awaited would make every caller
 *  async. Callers that must not proceed without the rules check for null and
 *  fail closed rather than treating an unloaded module as nothing to refuse. */
export function core(): Core | null {
	return loaded;
}

/** Loads the module in a browser, from the URL Vite emits for the asset. */
export async function loadCore(): Promise<Core> {
	if (loaded !== null) {
		return loaded;
	}
	loading ??= init({ module_or_path: wasmUrl }).then(() => {
		loaded = bound;
		return bound;
	});
	return loading;
}

/** Loads the module from bytes, which is how a test under node initialises it:
 *  there is no fetch to serve the asset URL there. */
export function loadCoreFrom(bytes: BufferSource): Core {
	if (loaded === null) {
		initSync({ module: bytes });
		loaded = bound;
	}
	return loaded;
}
