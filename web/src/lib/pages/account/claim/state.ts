// What the claim screen says about the slug in the field, as one function over
// the draft and the last settled availability probe.
//
// Lifted out of the markup deliberately. A verdict computed in a template is
// reachable only by rendering the page, and the case that matters most here --
// a probe that failed -- is the one a rendered assertion is least likely to
// reach. Held as a function, every case is a unit test costing milliseconds.

import type { SlugPrompt } from '$lib/generated/vocab';
import { checkOrgSlug } from '$lib/org-slug';

/** The last settled availability probe, as the screen knows it.
 *
 * `available` is `null` rather than `false` when nothing has been read. The two
 * are different facts and the screen renders them differently: a slug someone
 * else holds is a refusal the seller must act on, while a probe that did not
 * answer is a question still open, and folding the second into the first would
 * tell a seller their chosen name was taken because our own request failed. */
export interface Probe {
	/** The slug this probe judged, so a verdict for an earlier keystroke is
	 *  never rendered against the value now in the field. */
	slug: string | null;
	pending: boolean;
	available: boolean | null;
	failed: boolean;
}

export const NO_PROBE: Probe = { slug: null, pending: false, available: null, failed: false };

export type ClaimStatus =
	| { kind: 'empty' }
	| { kind: 'invalid'; message: string }
	| { kind: 'checking'; slug: string }
	| { kind: 'available'; slug: string; message: string }
	| { kind: 'taken'; slug: string; message: string }
	| { kind: 'unknown'; slug: string; message: string };

export function claimStatus(draft: string, probe: Probe): ClaimStatus {
	const verdict = checkOrgSlug(draft);
	if (!verdict.accepted) {
		return verdict.problem === 'empty'
			? { kind: 'empty' }
			: { kind: 'invalid', message: verdict.message };
	}
	const slug = verdict.slug;
	// A probe that judged a different slug says nothing about this one, and
	// neither does one still in flight.
	if (probe.slug !== slug || probe.pending) {
		return { kind: 'checking', slug };
	}
	if (probe.failed || probe.available === null) {
		return {
			kind: 'unknown',
			slug,
			message: 'We could not check that name. You can still save it.'
		};
	}
	return probe.available
		? { kind: 'available', slug, message: `${slug} is available.` }
		: { kind: 'taken', slug, message: 'That name is taken. Try another.' };
}

/** Why the claim cannot be submitted, or `null` when it can.
 *
 * A slug the probe could not judge is submittable, and so is one still being
 * checked. The probe is advisory by design -- the server's unique index is the
 * only arbiter -- so blocking on it would make an advisory check load-bearing
 * and would strand a seller whose network dropped one request. */
export function claimBlockedReason(status: ClaimStatus, saving: boolean): string | null {
	if (saving) {
		return 'Saving your name.';
	}
	switch (status.kind) {
		case 'empty':
			return 'Choose a name for your account.';
		case 'invalid':
			return status.message;
		case 'taken':
			return status.message;
		case 'checking':
		case 'unknown':
		case 'available':
			return null;
	}
}

/** Whether a slug is worth spending an availability probe on.
 *
 * The endpoint is bounded per session, so a probe on a slug the client already
 * knows is malformed spends budget to be told what it already knew. */
export function worthProbing(draft: string): string | null {
	const verdict = checkOrgSlug(draft);
	return verdict.accepted ? verdict.slug : null;
}

/** What the console shell should do about an unclaimed slug on this load. */
export type GateVerdict = 'claim-screen' | 'banner' | 'nothing';

/** Whether the claim screen stands in place of the console, a banner sits
 *  above it, or neither.
 *
 *  Lifted out of the layout for the same reason the field's status was lifted
 *  out of the markup: the case that matters most here is the one hardest to
 *  reach by rendering. An operator impersonating a tenant who has claimed no
 *  slug must not meet the gate, because the claim screen renders outside
 *  `Console` and `Console` is what carries the stop-impersonating control --
 *  so gating an impersonated session strands the operator inside a tenant with
 *  no way back out, and the one action they need is the one the gate removes.
 *  They see the banner and the ordinary console instead, and the tenant names
 *  their own organisation, which is whose name it is.
 *
 *  The public pages are outside the gate as well. `/status` in particular: it
 *  matters most when something is broken, and a seller who cannot reach it
 *  because they have not named their organisation is being kept from the one
 *  page that would tell them why. */
export function consoleGate(input: {
	prompt: SlugPrompt | undefined;
	impersonating: boolean;
	publicRoute: boolean;
	bannerDismissed: boolean;
}): GateVerdict {
	if (input.prompt === undefined || input.publicRoute) {
		return 'nothing';
	}
	if (input.prompt === 'settled') {
		return 'nothing';
	}
	if (input.prompt === 'claim' && !input.impersonating) {
		return 'claim-screen';
	}
	return input.bannerDismissed ? 'nothing' : 'banner';
}
