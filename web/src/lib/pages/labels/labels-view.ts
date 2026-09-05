// The Labels page's own view logic: the rule the rename field applies, the
// words a row and a delete confirmation carry, and the class one label's
// colour renders as. Pure, so it tests without a component.

import type { LabelView } from '$lib/api';
import type { LabelColour } from '$lib/generated/vocab';

/** The server's bound, counted in characters -- Unicode scalar values, which
 *  is what `str::chars().count()` counts -- rather than UTF-16 code units, so
 *  a name written in accented or non-Latin letters is measured the way the
 *  seller who typed it sees it. */
export const LABEL_MAX_CHARS = 60;

/** Why a rename was refused, in the cases a seller can reach by typing. */
export type NameProblem = 'empty' | 'too-long' | 'slash' | 'taken';

export type NameVerdict =
	| { accepted: true; name: string }
	| { accepted: false; problem: NameProblem; message: string };

/** The rename rule, mirroring `validated_label` and the rename's own `Taken`
 *  answer (`crates/tam-api/src/resources.rs:1053-1073,1187-1191`).
 *
 *  The check spares a round trip for a name the server would refuse; it does
 *  not stand in for the server's, and the field renders the 422 body whenever
 *  the two disagree. `existing` is the whole vocabulary including `current`,
 *  because a case-only rename updates the row it matched and so is not taken:
 *  the unique index is over `lower(name)` and the row conflicts with nothing
 *  but itself (`crates/tam-storage/src/labels.rs:254-269`).
 *
 *  The control-character rule the server also applies is deliberately not
 *  repeated here: it cannot be reached from a text field, and duplicating it
 *  would be a second spelling of a rule with no way to test it against the
 *  first. */
export function checkRename(
	raw: string,
	current: string,
	existing: readonly string[]
): NameVerdict {
	const name = raw.trim();
	if (name.length === 0) {
		return { accepted: false, problem: 'empty', message: 'A label needs a word in it.' };
	}
	// Spread rather than `.length`: the string iterator yields code points, so
	// this counts what the server counts.
	if ([...name].length > LABEL_MAX_CHARS) {
		return {
			accepted: false,
			problem: 'too-long',
			message: `A label is at most ${LABEL_MAX_CHARS} characters.`
		};
	}
	if (name.includes('/')) {
		return {
			accepted: false,
			problem: 'slash',
			message: 'A label cannot contain a slash, because a label is addressed by its own name.'
		};
	}
	const folded = name.toLowerCase();
	const held = current.trim().toLowerCase();
	if (folded !== held && existing.some((one) => one.trim().toLowerCase() === folded)) {
		return {
			accepted: false,
			problem: 'taken',
			message: 'That name is already one of your labels.'
		};
	}
	return { accepted: true, name };
}

/** A rename that changes nothing is not sent: the answer would be the label
 *  the page already holds, and the toast would claim a write that did not
 *  happen. Case is a change, because the seller's own capitalisation is what
 *  they see. */
export function unchanged(raw: string, current: string): boolean {
	return raw.trim() === current.trim();
}

/** One label as the page renders it. `count` is `null` while the walk that
 *  counts it is still running or after it failed, which the row says rather
 *  than filling in with a figure nothing measured. */
export interface LabelRow {
	name: string;
	colour: string;
	count: number | null;
}

export function rows(labels: readonly LabelView[], counts: ReadonlyMap<string, number>): LabelRow[] {
	return labels.map((label) => ({
		name: label.name,
		colour: label.colour,
		count: counts.get(label.name) ?? null
	}));
}

/** The search narrows what is shown rather than what was read: the whole
 *  vocabulary is one small answer, so a search that reached the server would
 *  cost a round trip to filter a list already in hand. */
export function matching(all: readonly LabelRow[], search: string): LabelRow[] {
	const term = search.trim().toLowerCase();
	if (term.length === 0) {
		return [...all];
	}
	return all.filter((row) => row.name.toLowerCase().includes(term));
}

/** The row's one meta line, in the three states the count can be in.
 *
 *  A count still being walked says so, and one whose walk failed says that
 *  instead: a label no resource carries cannot exist, so a "0 resources"
 *  standing in for an unread figure would be a lie the seller could act on. */
export function countLine(count: number | null, counting: boolean): string {
	if (count !== null) {
		return count === 1 ? '1 resource' : `${count} resources`;
	}
	return counting ? 'Counting…' : 'Count unavailable';
}

/** What the delete confirmation says before anything is removed.
 *
 *  The label is named rather than called "this label": the panel opens inside a
 *  list of rows that all look alike, and a sentence that names only a figure
 *  leaves the seller matching numbers to work out what they are about to
 *  destroy. The count is named for the same reason and degrades rather than
 *  guessing when it is not known. */
export function deleteWarning(name: string, count: number | null): string {
	if (count === null) {
		return `Deleting ${name} removes it from every resource that carries it.`;
	}
	if (count === 1) {
		return `${name} is on 1 resource. Deleting it removes it from that resource.`;
	}
	return `${name} is on ${count} resources. Deleting it removes it from all of them.`;
}

/** The eyebrow over the list. */
export function totalLine(count: number): string {
	return `Total labels: ${count}`;
}

/** The class each stored colour renders as.
 *
 *  A total map over the generated union rather than the colour name used as a
 *  class directly, so a colour added to the closed set in Rust stops this file
 *  type-checking instead of rendering an unstyled swatch. `LabelsDialog.svelte`
 *  holds the same map for the same reason. */
const SWATCH: Record<LabelColour, string> = {
	slate: 'c-slate',
	red: 'c-red',
	amber: 'c-amber',
	green: 'c-green',
	teal: 'c-teal',
	blue: 'c-blue',
	violet: 'c-violet',
	pink: 'c-pink'
};

export function swatchClass(colour: string): string {
	// `Object.hasOwn` rather than indexing straight in: an unknown colour that
	// happens to name a prototype member would otherwise answer that member
	// instead of falling back, and the fallback is the whole point of the call.
	return Object.hasOwn(SWATCH, colour) ? SWATCH[colour as LabelColour] : SWATCH.slate;
}

/** Where the Resources list opens showing only what carries this label. The
 *  name is one query value rather than a path segment, so a label carrying a
 *  reserved character survives the trip. */
export function filterHref(name: string): string {
	return `/inventory?label=${encodeURIComponent(name)}`;
}
