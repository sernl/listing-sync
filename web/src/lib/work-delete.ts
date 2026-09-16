// Deleting work history: what the seller is told, what the server's three
// answers mean, and what a part-finished run leaves selected. Pure, so it
// tests without a component.
//
// "Work" here is an import run, a migration or sync request, and a publishing
// run — three tables with one lifecycle. Deleting one removes it from the
// seller's own history once new work has been fenced; it removes no resource,
// no file and no marketplace listing, which is the whole reason this wording
// lives apart from the resource-deletion dialogs.

import type { JobDeletionStatus } from '$lib/api';

/** One row a Delete would act on: its identity on the server and the words
 *  the confirmation names it by.
 *
 *  The label travels with the id rather than being looked up at confirm
 *  time, because a selection survives turning the page and the row it came
 *  from may no longer be on screen. */
export interface WorkItem {
	id: string;
	label: string;
}

/** What one Delete did, item by item.
 *
 *  Four arms rather than a success count: the server's 200 and its 202 are
 *  different facts, `needs_review` is a row that stays visible on purpose,
 *  and a transport failure is the only one of the four the seller can act on
 *  again. */
export interface WorkDeleteOutcome {
	deleted: string[];
	stopping: string[];
	needsReview: string[];
	failed: { id: string; label: string; message: string }[];
}

/** The outcome with one server answer folded in. */
export function withStatus(
	outcome: WorkDeleteOutcome,
	id: string,
	status: JobDeletionStatus
): WorkDeleteOutcome {
	switch (status) {
		case 'deleted':
			return { ...outcome, deleted: [...outcome.deleted, id] };
		case 'stopping':
			return { ...outcome, stopping: [...outcome.stopping, id] };
		case 'needs_review':
			return { ...outcome, needsReview: [...outcome.needsReview, id] };
	}
}

/** The outcome with one refusal folded in. Carries the label as well as the
 *  id, because the sentence that reports it names the row rather than a
 *  UUID. */
export function withFailure(
	outcome: WorkDeleteOutcome,
	item: WorkItem,
	message: string
): WorkDeleteOutcome {
	return {
		...outcome,
		failed: [...outcome.failed, { id: item.id, label: item.label, message }]
	};
}

/** How many rows the server answered about at all, which is what decides
 *  between showing the choice and showing what happened. */
export function answered(outcome: WorkDeleteOutcome): number {
	return (
		outcome.deleted.length +
		outcome.stopping.length +
		outcome.needsReview.length +
		outcome.failed.length
	);
}

/** What a seller keeps ticked after a part-finished Delete.
 *
 *  Every row the server answered about is dropped — including a `stopping`
 *  and a `needs_review`, which were accepted and are not a second act for the
 *  seller to take. The refusals stay ticked, because those are the ones a
 *  retry is for and re-ticking them by hand is the one thing a half-finished
 *  bulk must not ask for. */
export function keptSelection(
	selected: ReadonlySet<string>,
	outcome: WorkDeleteOutcome
): Set<string> {
	const settled = new Set([...outcome.deleted, ...outcome.stopping, ...outcome.needsReview]);
	return new Set([...selected].filter((id) => !settled.has(id)));
}

/** Nouns for one kind of work, so one dialog can name three histories without
 *  the call sites spelling plurals. */
export interface WorkNoun {
	one: string;
	many: string;
}

export function countWord(count: number, noun: WorkNoun): string {
	return `${count} ${count === 1 ? noun.one : noun.many}`;
}

/** What Delete does and — more to the point — what it leaves alone.
 *
 *  Said in full on every one of these dialogs. A seller who has just read
 *  "delete" on a resource dialog, where it does remove marketplace listings,
 *  has every reason to expect the same here, and the two are not the same
 *  act. */
export const WORK_DELETE_KEEPS =
	'Everything this work already did stays as it is: resources already imported, the files ' +
	'on your computer and every marketplace listing are untouched. Deleting stops work that ' +
	'has not started yet and takes the record out of your history.';

/** The one caveat that is not about what is kept: a resource already in
 *  flight is not snatched back mid-write. */
export const WORK_DELETE_IN_FLIGHT =
	'Anything already being sent may finish before the stop takes effect. Nothing new starts ' +
	'after that.';

/** What each of the server's three answers means, said as the seller reads
 *  it rather than as the wire spells it. */
const STATUS_LINE: Record<JobDeletionStatus, string> = {
	deleted: 'gone from your history',
	stopping: 'stopping and will leave your history once the work already running has stopped',
	needs_review: 'kept for review until the marketplace results are confirmed'
};

/** What happened, one sentence per arm that has anything in it.
 *
 *  Never one rolled-up "deleted N": a 202 is not a deletion and a row kept
 *  for review is not one either, and a seller told otherwise would go looking
 *  for a row that is still there. */
export function outcomeLines(outcome: WorkDeleteOutcome, noun: WorkNoun): string[] {
	const lines: string[] = [];
	if (outcome.deleted.length > 0) {
		lines.push(`${countWord(outcome.deleted.length, noun)} ${outcome.deleted.length === 1 ? 'is' : 'are'} ${STATUS_LINE.deleted}.`);
	}
	if (outcome.stopping.length > 0) {
		lines.push(`${countWord(outcome.stopping.length, noun)} ${outcome.stopping.length === 1 ? 'is' : 'are'} ${STATUS_LINE.stopping}.`);
	}
	if (outcome.needsReview.length > 0) {
		lines.push(`${countWord(outcome.needsReview.length, noun)} ${outcome.needsReview.length === 1 ? 'is' : 'are'} ${STATUS_LINE.needs_review}.`);
	}
	for (const failure of outcome.failed) {
		lines.push(`Deletion of ${failure.label} is not confirmed. ${failure.message}`);
	}
	return lines;
}

/** The badge a row carries while it is being deleted but is still in the
 *  history, or null for a row that is simply itself.
 *
 *  `deleted` never reaches this: a deleted row is filtered out at the
 *  server's own pagination boundary, so the console never draws one. */
export function retainedBadge(
	status: JobDeletionStatus | null | undefined
): { tone: 'run' | 'warn'; label: string } | null {
	if (status === 'stopping') {
		return { tone: 'run', label: 'Stopping' };
	}
	if (status === 'needs_review') {
		return { tone: 'warn', label: 'Needs review' };
	}
	return null;
}

/** Why Delete is not offered on a row that is already going.
 *
 *  Repeating it is harmless on the server, which is idempotent, but a control
 *  that says "Delete" over a row that says "Stopping" reads as though the
 *  first press did nothing. */
export function deleteRefusal(status: JobDeletionStatus | null | undefined): string | null {
	if (status === 'stopping') {
		return 'This is already stopping. It leaves your history once the work already running has stopped.';
	}
	if (status === 'needs_review') {
		return 'This is kept for review until we can confirm what it wrote on a marketplace.';
	}
	return null;
}
