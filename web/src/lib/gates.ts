import type { BlockedGate } from '$lib/generated/vocab';

/**
 * What each gate means to the seller, in their words rather than ours.
 *
 * Typed as an exhaustive record over the generated union, so a gate added in
 * Rust with no label here fails `svelte-check` rather than reaching a seller
 * as a raw identifier. The union itself is regenerated from `ALL_GATES` and
 * the web lane fails when it drifts.
 */
const LABELS: Record<BlockedGate, string> = {
	reconciliation: 'reconciliation',
	election: 'your answer',
	currency_unknown: 'an unknown currency',
	cover_missing: 'a missing cover image',
	scan_incomplete: 'a scan that is still running',
	awaiting_counterpart: 'the other marketplace',
	binding: 'a listing that already exists here',
	unbound: 'a listing that does not exist here yet',
	subject_diverged: 'a listing that changed on the marketplace',
	lifecycle_diverged: 'a listing in an unexpected state',
	ReauthRequired: 'you signing in again',
	awaiting_seller_signin: 'you signing in again, so this can be checked'
};

/** The label for a gate, falling back to the raw value for an unknown one. */
export function gateLabel(gate: string): string {
	return LABELS[gate as BlockedGate] ?? gate;
}
