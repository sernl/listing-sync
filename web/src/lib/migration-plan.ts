// What the Migrations page can say before it asks the server anything: which
// pairs of marketplaces a migration is offered between, the words the console
// uses for Copy and Move and for each verdict, and the allowance sentence over
// the confirm. Pure, so it tests without a component.
//
// The pair table is derived rather than listed: an inventory is refused as a
// source because Teachouse has captured no download for it, and refused as a
// target because no adapter writes to it. Both facts already live in this
// bundle for other reasons, so a new marketplace either arrives with them
// answered or fails the type check here.

import type { Disposition, MigrationBody, MigrationCap, MigrationCounts } from '$lib/api';
import { movesReason } from '$lib/entitlement';
import type { InventoryId, MigrationVerdict } from '$lib/generated/vocab';
import { INVENTORY_ORDER } from '$lib/listings-view';
import { AUTHORABLE, SHORT_NAME, platformTitle } from '$lib/platforms';

/** The verdict as a chip reads it. A total map, so a verdict added on the
 *  server and regenerated into the vocabulary is worded here or fails the type
 *  check rather than rendering an empty chip. */
export const VERDICT_WORD: Record<MigrationVerdict, string> = {
	will_create: 'Will create',
	already_there: 'Already there',
	blocked: 'Blocked'
};

/** The tone each verdict is drawn in. `already_there` is grey rather than
 *  green: nothing is wrong with it and nothing is going to happen to it,
 *  which is what grey means everywhere else in this console. */
export const VERDICT_TONE: Record<MigrationVerdict, 'ok' | 'soon' | 'bad'> = {
	will_create: 'ok',
	already_there: 'soon',
	blocked: 'bad'
};

// ------------------------------------------------------------- copy and move

/** The console's word for each disposition. The wire spells them `sync` and
 *  `migrate`; a seller reads Copy and Move. */
export const DISPOSITION_WORD: Record<Disposition, string> = {
	sync: 'Copy',
	migrate: 'Move'
};

/** What each one does to the source listing, which is the whole difference and
 *  the only thing a seller has to decide. */
export const DISPOSITION_LINE: Record<Disposition, string> = {
	sync: 'Copy leaves the source listing where it is.',
	migrate:
		'Move creates the listing on the target, then removes it from the source once it is there.'
};

/** Both, in the order the segmented control shows them: the safe one first. */
export const DISPOSITIONS: readonly Disposition[] = ['sync', 'migrate'];

/** The verb a confirm button leads with, agreeing with its count. */
export function confirmLabel(disposition: Disposition, count: number): string {
	return `${DISPOSITION_WORD[disposition]} ${count} ${count === 1 ? 'resource' : 'resources'}`;
}

// -------------------------------------------------------------- the pair table

/** Which download this marketplace has no capture for, or null where a
 *  migration can read it.
 *
 * Mirrors `tam_storage::uncaptured_source`: TES and TPT have captured
 * device-local downloads; Etsy remains unavailable as a source.
 * A total map rather than a list, so an inventory added in Rust stops this
 * file type-checking instead of being offered as a source nothing can read. */
export const UNCAPTURED_SOURCE: Record<InventoryId, string | null> = {
	Tes: null,
	Tpt: null,
	Etsy: 'etsy.download_resource_bundle'
};

/** One end of a migration as the select offers it: always present, sometimes
 *  disabled, and never disabled without saying why. A marketplace left out of
 *  the list altogether reads as one Teachouse has never heard of, which is a
 *  different and false statement. */
export interface MigrationSide {
	inventory: InventoryId;
	label: string;
	enabled: boolean;
	reason: string | null;
}

function sourceReason(inventory: InventoryId): string | null {
	return UNCAPTURED_SOURCE[inventory] === null
		? null
		: `Teachouse cannot yet download files from ${SHORT_NAME[inventory]}, so it cannot be a source.`;
}

function targetReason(inventory: InventoryId): string | null {
	return AUTHORABLE[inventory]
		? null
		: `${SHORT_NAME[inventory]} is not connected to Teachouse yet.`;
}

function sidesBy(reason: (inventory: InventoryId) => string | null): MigrationSide[] {
	return INVENTORY_ORDER.map((inventory) => {
		const refusal = reason(inventory);
		return {
			inventory,
			label: platformTitle(inventory),
			enabled: refusal === null,
			reason: refusal
		};
	});
}

/** Every marketplace as a From option. */
export function migrationSources(): MigrationSide[] {
	return sidesBy(sourceReason);
}

/** Every marketplace as a To option. */
export function migrationTargets(): MigrationSide[] {
	return sidesBy(targetReason);
}

/** The sources a migration can actually read, which is what other surfaces
 *  ask for when they want to know whether the seller has a shop to bring
 *  across at all. */
export const MIGRATION_SOURCES: readonly InventoryId[] = INVENTORY_ORDER.filter(
	(inventory) => UNCAPTURED_SOURCE[inventory] === null
);

/** Why this pair cannot be migrated between, or null where it can.
 *
 * Ordered so the seller reads the thing they would change first: two ends that
 * are the same marketplace is a slip, and a capture gap is not. Both ends are
 * checked even when the first already refuses, because the sentence names one
 * cause and the seller fixes one thing at a time. */
export function pairReason(source: InventoryId, target: InventoryId): string | null {
	if (source === target) {
		return `A migration moves resources between two marketplaces, and both ends here are ${SHORT_NAME[source]}.`;
	}
	return sourceReason(source) ?? targetReason(target);
}

// ------------------------------------------------------------- the sentences

/** The counts line under the preview: "8 will be created, 3 already there, 1
 *  blocked". Every figure is printed, including the zeroes, because a missing
 *  clause reads as a category the preview did not look at. */
export function countsLine(counts: MigrationCounts): string {
	return (
		`${counts.will_create} will be created, ` +
		`${counts.already_there} already there, ` +
		`${counts.blocked} blocked`
	);
}

/** The balance sentence and the refusal, read off the plan's own `cap` block
 *  rather than off the entitlement read.
 *
 * One writer, `movesReason`: the figure over the confirm and the figure the
 * submit is checked against have to be the same figure, and the plan's cap is
 * the fresher of the two. */
export function capSentence(
	cap: MigrationCap,
	requested: number
): { line: string; refusal: string | null } {
	return movesReason({ available: cap.available }, requested);
}

// ------------------------------------------------------------------ the body

/** A plan request and a submit, which are the same body.
 *
 *  There is no `intent`: a migration always lands as a draft, because
 *  publishing is a decision the seller takes over what arrived rather than one
 *  taken blind beforehand. An empty tick list stays a tick list rather than
 *  collapsing to "all the source holds", which would move a shop nobody asked
 *  to move. */
export function migrationBody(input: {
	source: InventoryId;
	target: InventoryId;
	disposition: Disposition;
	all: boolean;
	products: readonly string[];
}): MigrationBody {
	return {
		source: input.source,
		target: input.target,
		disposition: input.disposition,
		selection: input.all ? { all: true } : { products: [...input.products] }
	};
}

/** The resources a link into this page asks to be ticked already.
 *
 * The Resources board sends a selection here as `?products=<comma ids>` rather
 * than carrying it in memory, because the two are separate pages and a
 * selection worth acting on is worth being able to reopen. Blanks and repeats
 * are dropped and the order is the address's own: a hand-edited URL preselects
 * what it can rather than refusing the page. An identifier this organisation
 * does not hold is not filtered here — the checklist only ticks what it can
 * find, so an unknown identifier ticks nothing. */
export function productsFromUrl(params: URLSearchParams): string[] {
	const seen = new Set<string>();
	for (const raw of params.getAll('products')) {
		for (const part of raw.split(',')) {
			const id = part.trim();
			if (id.length > 0) {
				seen.add(id);
			}
		}
	}
	return [...seen];
}
