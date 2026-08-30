// What the publish dialog can honestly say about one platform before anything
// is enqueued, computed from what this org's own data model holds and from no
// marketplace contact at all. Pure, so it tests without a component.
//
// Two things are mirrored from the server rather than invented: the lowering
// table, which decides whether a transition can be attempted at all, and the
// registry's requiredness, which decides whether a create would be refused.
// Both stay predictions — the engine re-projects at seed time and can park on
// an election raised there — so the dialog says what it knows and never that
// the write will succeed.

import type { ConnectionView, InventoryStatus, MappingHead, VocabularyView } from '$lib/api';
import type { PublishIntent } from '$lib/api';
import { platformTitle } from '$lib/platforms';
import { MARKETPLACE_OF } from '$lib/listings-view';
import type { InventoryId } from '$lib/generated/vocab';

/** The two states a listing is asked to end up in, which is exactly the
 *  server's own `ListingState` and not a second vocabulary beside it. */
type ListingState = 'draft' | 'live';

const STATE_OF: Record<PublishIntent, ListingState> = { draft: 'draft', live: 'live' };

/** Which transition this inventory has no capture for, if any.
 *
 * Mirrors `tam_storage::uncaptured_transition`: Tes implements only the two
 * transitions out of draft, so a listing already live there can be neither
 * edited in place nor taken back to draft by us. TPT serves all four. */
export function uncapturedTransition(
	inventory: InventoryId,
	from: ListingState,
	to: ListingState
): string | null {
	if (MARKETPLACE_OF[inventory] !== 'Tes' || from !== 'live') {
		return null;
	}
	return to === 'live' ? 'tes.edit_published' : 'tes.unpublish';
}

/** Why the lowering would refuse this mapping, or `null` where it lowers.
 *
 * Mirrors `tam_storage::lower` over the two states a mapping head carries.
 * An unrecognised binding state is a create in flight or one whose outcome
 * nobody knows, and either way there is no binding to lower against. */
export function loweringRefusal(
	mapping: MappingHead,
	intent: PublishIntent
): string | null {
	const to = STATE_OF[intent];
	if (mapping.binding_state === 'unbound' || mapping.binding_state === 'severed') {
		return null;
	}
	if (mapping.binding_state !== 'bound') {
		return 'a send is already in flight for this platform; wait for it to settle';
	}
	if (mapping.lifecycle_state !== 'draft' && mapping.lifecycle_state !== 'live') {
		return 'we have not recorded what this listing currently is, so nothing can be sent yet';
	}
	const capability = uncapturedTransition(mapping.inventory, mapping.lifecycle_state, to);
	if (capability === null) {
		return null;
	}
	return to === 'live'
		? 'this listing is already live here, and editing a published listing is uncaptured'
		: 'this listing is live here, and taking a published listing back to draft is uncaptured';
}

/** Everything one readiness line is computed from. Each field is a fact this
 *  client already holds for another reason; nothing here is fetched to answer
 *  the question. */
export interface ReadinessInput {
	inventory: InventoryId;
	intent: PublishIntent;
	mapping: MappingHead | undefined;
	connection: ConnectionView | undefined;
	status: InventoryStatus | undefined;
	vocabulary: VocabularyView | undefined;
	/** How many payload files the product carries, against the platform's own
	 *  payload rule. */
	payloadFiles: number;
	/** Whether the product carries a rights grant, which is the one field the
	 *  registry declares required anywhere. */
	hasRights: boolean;
}

export interface Readiness {
	inventory: InventoryId;
	title: string;
	ready: boolean;
	/** The sentence beneath the tick box. One clause, in the seller's terms. */
	line: string;
	tone: 'ok' | 'run' | 'bad' | 'mut';
}

/** The first required field this product does not answer, or `null` where it
 *  answers them all.
 *
 * Mirrors the server's own `required_fields_answered`: requiredness is
 * reported as the registry has it, a licence reaches the write as the
 * product's rights declaration, and a required field nothing has been taught
 * to answer reads as unanswered rather than as silently satisfied. */
function unansweredRequired(view: VocabularyView, hasRights: boolean): string | null {
	for (const native of view.natives) {
		if (!native.required) {
			continue;
		}
		const axis = view.axes.find((binding) => binding.native === native.name);
		if (axis?.axis === 'licence' && hasRights) {
			continue;
		}
		return native.name;
	}
	return null;
}

/** The first thing standing between this product and this platform, or `null`
 *  where nothing this client can see does.
 *
 * Ordered so the seller reads the cause they would fix first: a platform with
 * no connection cannot be fixed by adding a licence. */
function blocker(input: ReadinessInput): { line: string; tone: 'bad' | 'run' } | null {
	if (input.mapping === undefined) {
		return {
			line: 'not one of this product’s platforms; platforms are chosen when the draft is created',
			tone: 'bad'
		};
	}
	if (input.connection === undefined) {
		return { line: 'no account is linked for this marketplace yet', tone: 'bad' };
	}
	if (input.connection.status === 'disconnected') {
		return { line: 'the linked account holds nothing usable; re-link it first', tone: 'bad' };
	}
	if (input.status?.halted === true) {
		const reason = input.status.reason;
		return {
			line:
				reason === undefined
					? 'sending is paused for this platform'
					: `sending is paused for this platform: ${reason}`,
			tone: 'run'
		};
	}
	const refusal = loweringRefusal(input.mapping, input.intent);
	if (refusal !== null) {
		return { line: refusal, tone: 'bad' };
	}
	if (input.vocabulary !== undefined) {
		const rule = input.vocabulary.authoring.payload_files;
		if (rule === 'exactly_one' && input.payloadFiles !== 1) {
			return {
				line: `takes exactly one file, and this product carries ${input.payloadFiles}`,
				tone: 'bad'
			};
		}
		const missing = unansweredRequired(input.vocabulary, input.hasRights);
		if (missing !== null) {
			return { line: `needs a ${missing}`, tone: 'run' };
		}
	}
	if (input.connection.status === 'unstable') {
		return {
			line: 'the linked account is failing verification; work may sit queued',
			tone: 'run'
		};
	}
	return null;
}

/** One platform's readiness line.
 *
 * "Ready" here means nothing this client holds refuses the send, not that the
 * marketplace will accept it: no marketplace was contacted and the engine
 * re-projects when it picks the work up. */
export function readinessOf(input: ReadinessInput): Readiness {
	const title = platformTitle(input.inventory);
	if (input.vocabulary === undefined) {
		return {
			inventory: input.inventory,
			title,
			ready: false,
			line: 'checking what this platform requires…',
			tone: 'mut'
		};
	}
	const found = blocker(input);
	if (found === null) {
		return { inventory: input.inventory, title, ready: true, line: 'ready to send', tone: 'ok' };
	}
	return { inventory: input.inventory, title, ready: false, line: found.line, tone: found.tone };
}

/** The connection a platform's writes travel over, matched by marketplace
 *  because a connection is held per marketplace and not per inventory. */
export function connectionFor(
	inventory: InventoryId,
	connections: readonly ConnectionView[]
): ConnectionView | undefined {
	return connections.find(
		(connection) => connection.marketplace === MARKETPLACE_OF[inventory]
	);
}
