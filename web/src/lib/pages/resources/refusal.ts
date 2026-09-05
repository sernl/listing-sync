// What a refused write reads as on the Resources pages. Pure, so it tests
// without a component — which is the point: the two faults these functions
// exist to prevent are both invisible from the screen until they happen.

import { ApiFailure } from '$lib/api';
import { quotaSentence, requiredFieldSentence } from '$lib/authoring';

/**
 * A refusal in the seller's words, or the page's own sentence where the
 * response carried nothing to quote.
 *
 * `ApiFailure.message` falls back to the literal `request failed with 502`
 * when the body did not parse — a version-skewed 404, a proxy's HTML error, a
 * rate limit from something in between — and a status line is not a sentence a
 * teacher can act on.
 */
export function sentenceFor(failure: ApiFailure, own: string): string {
	return failure.body === null ? own : failure.message;
}

export const NOT_CREATED = 'The draft was not created.';

/**
 * Why a create was refused.
 *
 * The index is `errors?.[0]` and not `errors[0]`. A body that parses but
 * carries no `errors` — a bare `{}` — is truthy, so the optional chain does not
 * short-circuit and the index throws a `TypeError`; that throw happens inside
 * the create's own catch block, where it is not caught again, so the refusal is
 * never assigned and the seller sees the page exactly as it was before they
 * pressed Create.
 */
export function createRefusal(failure: unknown): string {
	if (!(failure instanceof ApiFailure)) {
		return NOT_CREATED;
	}
	const entry = failure.body?.errors?.[0];
	const said = sentenceFor(failure, NOT_CREATED);
	switch (failure.code()) {
		case 'quota_exceeded':
			return quotaSentence(entry?.detail) ?? entry?.message ?? said;
		case 'payload_missing':
			return 'Upload your file before creating the draft.';
		// The server names the marketplace and the field in `detail.missing` and
		// always has; this case is the reason a seller creating for Tes read only
		// "a selected platform requires a field this product does not carry",
		// which names neither and cannot be acted on.
		case 'required_field_missing':
			return requiredFieldSentence(entry?.detail) ?? said;
		case 'upload_rejected':
			return `${said} Upload the file again.`;
		default:
			return said;
	}
}
