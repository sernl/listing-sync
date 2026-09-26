// One import run as a model: which of its six steps it is standing on, the
// one line its bar says, and the two sides of a pair it is asking about.
// Pure, so it tests without a component.
//
// A run is one shape for two sources. A spreadsheet run reviews a batch that
// was parsed on our own server; a marketplace run reviews a shop the seller's
// own computer read. What is true of both lives here; what is true only of
// the landing screen lives in `import-view`, and what is true only of a
// spreadsheet batch stays in `sheet-view`.

import type {
	ImportRunCounts,
	ImportRunHead,
	ImportRunItemView,
	ObservedPrice,
	ReviewPairView,
	ReviewSideView
} from '$lib/api';
import type { ImportReasonCode, InventoryId, Marketplace } from '$lib/generated/vocab';
import { MARKETPLACE_OF, formatPrice } from '$lib/listings-view';
import { platformTitle } from '$lib/platforms';
import { SYSTEM_LABEL } from '$lib/system-labels';
import type { PillTone } from './import-view';

// --------------------------------------------------------------------- stage

/** Presentation of the server's authoritative lifecycle and execution facts. */
export type RunStage =
	'waiting' | 'listing' | 'selecting' | 'reading' | 'reviewing' |
	'confirming' | 'committing' | 'interrupted' | 'done' | 'failed';

export function stageFrom(run: ImportRunHead): RunStage {
	switch (run.execution.stage) {
		case 'discovering':
			return 'listing';
		case 'completed':
			return 'done';
		case 'abandoned':
			return 'failed';
		case 'committing':
			return run.execution.commit_authorised ? 'committing' : 'confirming';
		default:
			return run.execution.stage;
	}
}

export interface StageBadge {
	tone: PillTone;
	label: string;
}

const BADGE: Record<RunStage, StageBadge> = {
	waiting: { tone: 'soon', label: 'Waiting for a device' },
	listing: { tone: 'run', label: 'Finding resources' },
	selecting: { tone: 'warn', label: 'Choose resources' },
	reading: { tone: 'run', label: 'Importing' },
	reviewing: { tone: 'warn', label: 'Needs you' },
	confirming: { tone: 'warn', label: 'Ready to confirm' },
	committing: { tone: 'run', label: 'Adding to Resources' },
	interrupted: { tone: 'warn', label: 'Paused' },
	done: { tone: 'ok', label: 'Imported' },
	failed: { tone: 'bad', label: 'Stopped' }
};

export function runBadge(run: ImportRunHead): StageBadge {
	if (run.state === 'complete' && (run.counts.failed > 0 || run.counts.skipped > 0)) {
		return { tone: 'warn', label: 'Finished, some left out' };
	}
	return BADGE[stageFrom(run)];
}

/** What the page leads with, and the line under it. */
export interface StageCopy {
	headline: string;
	detail: string;
}

const STAGE_COPY: Record<RunStage, StageCopy> = {
	waiting: {
		headline: 'Waiting for the Teachouse app.',
		detail: 'Open the app on a computer signed in to this marketplace. Nothing has started yet.'
	},
	interrupted: {
		headline: 'This import has paused.',
		detail: 'Open the app on the computer that was importing, then resume. Resources already added stay in Resources.'
	},
	confirming: {
		headline: 'Ready to add to Resources.',
		detail: 'Check the results below, then confirm. Importing does not publish anything.'
	},
	listing: {
		headline: 'Finding what is in your shop.',
		detail: 'Your computer is finding every resource in your shop. Next, you choose what to import.'
	},
	selecting: {
		headline: 'Choose what to import.',
		detail: 'Tick the resources you want in Resources. We skip the rest.'
	},
	reading: {
		headline: 'Importing the resources you chose.',
		detail: 'You can leave this page. The import carries on and this list fills in as it goes.'
	},
	reviewing: {
		headline: 'Check these resources before adding them.',
		detail: 'Answer any duplicate questions below, then confirm what is ready.'
	},
	committing: {
		headline: 'Adding them to Resources.',
		detail: 'We are adding the resources you approved. You can close this page.'
	},
	done: {
		headline: 'This import is finished.',
		detail: 'Everything below is now in Resources, except the ones marked Left out.'
	},
	failed: {
		headline: 'This import did not finish.',
		detail: 'What it did import is in Resources. Each resource below shows what happened.'
	}
};

export function stageCopy(stage: RunStage): StageCopy {
	return STAGE_COPY[stage];
}


/** The line the listing draws under a run's name.
 *
 * A settled run says what it did; a running one says how far it got. Neither
 * invents a total: a run whose enumeration has not landed has no denominator
 * and says so in words rather than in a zero. */
export function countsLine(counts: ImportRunCounts, readTotal: number | null): string {
	if (readTotal === null) {
		return 'Not counted yet.';
	}
	const said: string[] = [`${readTotal} found`];
	if (counts.imported > 0) {
		said.push(`${counts.imported} imported`);
	}
	if (counts.review > 0) {
		said.push(`${counts.review} waiting on you`);
	}
	if (counts.skipped > 0) {
		said.push(`${counts.skipped} left out`);
	}
	if (counts.failed > 0) {
		said.push(`${counts.failed} with problems`);
	}
	return said.join(' · ');
}

// ------------------------------------------------------------------- listing

/** Where a run is read.
 *
 * A spreadsheet run is read on its batch's own page, which holds the report
 * its rows came from; the run is the review inside it rather than a second
 * screen. A marketplace run has no batch and is read on its own. */
export function runHref(run: ImportRunHead): string {
	if (run.kind === 'spreadsheet' && run.batch_id !== null) {
		return `/imports/${encodeURIComponent(run.batch_id)}`;
	}
	return `/imports/runs/${encodeURIComponent(run.id)}`;
}

/** What a run is called in a list of runs. The shop for a marketplace run,
 *  and the word for the other way in otherwise. */
export function runName(run: ImportRunHead): string {
	return run.source === null ? 'Spreadsheet' : platformTitle(run.source);
}

// --------------------------------------------------------------------- items

/** The badge one item carries in the list on a run's page. */
const ITEM_BADGE: Record<ImportRunItemView['state'], StageBadge> = {
	listed: { tone: 'flat', label: 'Found' },
	selected: { tone: 'run', label: 'Chosen' },
	read: { tone: 'run', label: 'Read' },
	matched: { tone: 'run', label: 'Ready' },
	review: { tone: 'warn', label: 'Needs you' },
	imported: { tone: 'ok', label: 'In Resources' },
	skipped: { tone: 'soon', label: 'Left out' },
	failed: { tone: 'bad', label: 'Problem' }
};

/** One resource as the run's list shows it. */
export interface ItemRow {
	locator: string;
	ordinal: number;
	/** Where the resource stands. Carried rather than read back off the
	 *  badge, because the tick list decides what can be chosen from it and a
	 *  comparison against a label would break the moment the wording did. */
	state: ImportRunItemView['state'];
	/** What to call it. The title where one was read, and the locator where
	 *  the enumeration named nothing else: an invented name would be a claim
	 *  about a listing nobody has opened. */
	name: string;
	price: string | null;
	coverUrl: string | null;
	label: string;
	tone: PillTone;
	/** Why it was left out, or what went wrong. Empty where neither. */
	reason: string;
	/** The resource it became, once it is one. */
	product: string | null;
}

function rowOf(item: ImportRunItemView): ItemRow {
	const badge = ITEM_BADGE[item.state] ?? { tone: 'soon', label: 'Unknown' };
	return {
		locator: item.locator,
		ordinal: item.ordinal,
		state: item.state,
		name: item.title ?? item.locator,
		price: item.price === null ? null : formatPrice({ Paid: item.price }),
		coverUrl: item.cover_url,
		label: badge.label,
		tone: badge.tone,
		reason: item.skip_reason ?? item.failure_detail ?? '',
		product: item.product_id
	};
}

/** The rows one page of a run's resources draws.
 *
 * An item array rather than the whole run, because the items are now a page
 * the server cut and the whole run is what `counts` and `read_total` still
 * speak for. A model that took the run would quietly invite a caller to
 * count the page and call it the run. */
export function itemRows(items: readonly ImportRunItemView[]): ItemRow[] {
	return items.map(rowOf);
}

/** The resources on this page the seller is being asked to tick.
 *
 * Only the listed ones: a resource already read, skipped or imported is not
 * a choice. The tick list itself is held by locator on the page, so a
 * resource ticked here stays ticked on page four. */
export function selectionRows(items: readonly ImportRunItemView[]): ItemRow[] {
	return items.filter((item) => item.state === 'listed').map(rowOf);
}

/** Why the Continue control cannot be pressed, or null where it can.
 *
 * A disabled control states its reason, which is what `Button` requires of
 * every caller; and the reason here is a thing the seller does rather than a
 * thing that is wrong, so it reads as an instruction. */
export function selectionBlocked(chosen: number, sending: boolean): string | null {
	if (sending) {
		return 'Sending your choice.';
	}
	return chosen === 0 ? 'Tick at least one resource to import.' : null;
}

// -------------------------------------------------------------------- review

/** One side of a review card, as the page draws it. */
export interface ReviewSide {
	/** Which side of the pair this is, which is what a verdict names. */
	side: 'lo' | 'hi';
	title: string;
	price: string;
	grades: string;
	marketplace: Marketplace | null;
	coverUrl: string | null;
	/** The resource this side is, where it is one already. Null where it is a
	 *  resource this run read and has not created. */
	product: string | null;
	/** What this side is: already in Resources, or only just read. Said
	 *  plainly, because keeping the one that does not exist yet and keeping
	 *  the one that does are different acts. */
	standing: string;
}

/** One pair as a card. */
export interface ReviewCard {
	/** The pair's own handle, which is what a verdict is posted against. */
	lo: string;
	hi: string;
	/** The one sentence of evidence, the server's own. Never a score. */
	sentence: string;
	sides: [ReviewSide, ReviewSide];
}

/** What one side says it is. The distinction matters: a seller keeping the
 *  side that is only a read is asking for it to be created, and a seller
 *  keeping the other is asking for nothing to be. */
const IN_RESOURCES = 'Already in Resources';
const JUST_READ = 'New from your shop';

function sideOf(view: ReviewSideView, side: 'lo' | 'hi'): ReviewSide {
	return {
		side,
		title: view.title,
		price: priceLabel(view.price),
		grades: view.grades.join(', '),
		marketplace: view.marketplace,
		coverUrl: view.cover_url,
		product: view.product_id,
		standing: view.product_id === null ? JUST_READ : IN_RESOURCES
	};
}

/** A price the read did not carry is an em dash rather than "Free": a
 *  marketplace that told us nothing has not told us a resource is free. */
function priceLabel(price: ObservedPrice | null): string {
	return price === null ? '—' : formatPrice({ Paid: price });
}

export function reviewCard(pair: ReviewPairView): ReviewCard {
	return {
		lo: pair.product_lo,
		hi: pair.product_hi,
		sentence: pair.sentence,
		sides: [sideOf(pair.lo, 'lo'), sideOf(pair.hi, 'hi')]
	};
}

/** The three answers a card admits, in the order the design names them. */
export const REVIEW_SAME = 'Same resource, keep one';
export const REVIEW_DIFFERENT = 'Different resources';
export const REVIEW_LATER = 'Decide later';

/** The fields a merge asks about, and what each is called.
 *
 * Three rather than every field of a listing: these are the three a seller
 * can read side by side and tell apart at a glance, and the rest follow the
 * one they keep. */
export const MERGE_FIELDS: readonly {
	field: 'title' | 'description' | 'price';
	label: string;
}[] = [
	{ field: 'title', label: 'Title' },
	{ field: 'description', label: 'Description' },
	{ field: 'price', label: 'Price' }
];

/** What the merge step says above the field choices. */
export const WHICH_SIDE_WINS =
	'You keep one of these. Choose which one, then pick the wording for each field.';

/** How long a merge can be undone for, said as the deadline the seller was
 *  given rather than as a policy. */
export const MERGE_IS_REVERSIBLE =
	'You can undo a merge for thirty days. Nothing is deleted from your marketplaces.';

// ------------------------------------------------------------------- settled

/** Where the resources this run created are, filtered to the label it gave
 *  them. A marketplace run labels every resource with the shop it came from,
 *  so the summary can hand the seller exactly what it made rather than the
 *  whole catalogue. */
export function importedHref(source: InventoryId | null): string {
	if (source === null) {
		return '/resources';
	}
	return `/resources?label=${encodeURIComponent(SYSTEM_LABEL[MARKETPLACE_OF[source]])}`;
}

/** What the finished run says it did, in one line.
 *
 * States a zero rather than staying silent: a seller who imported nothing has
 * asked a question this sentence answers, and an absent line would read as a
 * page that did not finish loading. */
export function settledLine(counts: ImportRunCounts): string {
	const imported = `${counts.imported} ${counts.imported === 1 ? 'resource is' : 'resources are'} in Resources`;
	const rest: string[] = [];
	if (counts.skipped > 0) {
		rest.push(`${counts.skipped} left out`);
	}
	if (counts.failed > 0) {
		rest.push(`${counts.failed} did not import`);
	}
	return rest.length === 0 ? `${imported}.` : `${imported}, ${rest.join(' and ')}.`;
}

/** What the run page says while it holds no resources at all, which depends
 *  on why it holds none. */
export function emptyItemsLine(stage: RunStage): string {
	switch (stage) {
		case 'waiting':
			return 'Waiting for the app to find your first resource.';
		case 'interrupted':
			return 'No resources were found before this import paused.';
		case 'listing':
			return 'No resources found yet.';
		case 'selecting':
		case 'reading':
		case 'reviewing':
		case 'confirming':
		case 'committing':
			return 'Nothing has arrived yet.';
		case 'done':
			return 'Your shop had nothing to import.';
		case 'failed':
			return 'Nothing was found before this import stopped.';
	}
}

/** What the page says when a filter is on and nothing matched.
 *
 * Distinct from [`emptyItemsLine`], which answers "this run holds nothing":
 * a seller who searched and found nothing has narrowed a list that still has
 * things in it, and telling them the import is empty would be false. */
export const NO_ITEM_MATCHES =
	'No resource matches your search. Clear it to see the rest.';

/** Why an import stopped, in words that name what the seller does next.
 *
 * The server sends a code and, sometimes, a sentence of its own. The code is
 * what this reads: a transport diagnostic like `lease_expired` is true and
 * unreadable, and a seller shown it learns nothing they can act on. The
 * server's own sentence is kept only where there is no code to read, so
 * nothing is lost where the machinery said something this table cannot. */
export function reasonLine(code: ImportReasonCode | null, reason: string | null): string | null {
	switch (code) {
		case 'missing_session':
			return 'This computer is not signed in to that marketplace. Sign in from Marketplaces, then start the import again.';
		case 'not_permitted':
			return 'Your plan does not include importing from this marketplace. Nothing was changed.';
		case 'unsupported_source':
			return 'You cannot import from this marketplace yet. Nothing was changed.';
		case 'enumeration_failed':
			return 'We could not find the resources in your shop. The marketplace may be slow or may have signed you out. Try again from Import.';
		case 'description_failed':
			return 'Some resources could not be opened. What we got is below; bring in the rest with a new import.';
		case 'submission_failed':
			return 'What this computer found did not reach us. Nothing was added. Start the import again.';
		case 'activation_expired':
			return 'No computer started this import in time. Start it again from Import.';
		case 'lease_expired':
			return 'The computer running this import stopped responding. The app may be closed or the computer asleep. Resume it here, or start a new import.';
		case 'stopped':
			// Never "nothing was created": a stop during the commit leaves the
			// resources already added standing, and telling a seller otherwise
			// would send them looking for a catalogue they do have. The summary
			// beside this line is what says how many.
			return 'You stopped this import. Anything already added stays in Resources, and nothing more will be imported.';
		case 'client_update_required':
			return 'The Teachouse app on this computer is too old for this import. Update it, then start the import again.';
		case null:
			return reason;
	}
}

/** Whether the reason a run carries describes something wrong *now*.
 *
 * A run keeps the last reason it was given, and a reason that was true once
 * stays on the record after the run has recovered from it: a lease that
 * expired while a device slept is still recorded on the run the seller is now
 * reviewing. Presence of a reason is therefore no evidence of a current
 * failure, and only the run's own state can say whether the condition still
 * holds. So this reads the state and never the reason: while a device owns
 * the run, what it last reported is what is happening; once the run has
 * reached a stage we run ourselves, that report is history and belongs behind
 * the diagnostics disclosure rather than in the bar. */
export function deviceConditionIsCurrent(run: ImportRunHead): boolean {
	switch (run.state) {
		// A device holds this run: waiting to be picked up, listing the shop,
		// reading it, or paused part-way through. Whatever it last reported is
		// the run's present condition.
		case 'reading':
			return true;
		// Settled by a condition rather than by a seller finishing, so the
		// reason is the thing the page is there to explain.
		case 'failed':
		case 'abandoned':
			return true;
		// Past device execution: the read produced enough to review, and the
		// review, the confirmation and the commit are ours rather than a
		// device's, so an earlier device condition cannot be what is happening.
		case 'reviewing':
		case 'committing':
		case 'complete':
			return false;
	}
}

/** How many pages a list of this size is cut into, at this page size.
 *
 * At least one, so a list with nothing in it reads as "Page 1 of 1" rather
 * than as a pager that has lost its place. */
export function pageCount(total: number, size: number): number {
	return Math.max(1, Math.ceil(total / Math.max(1, size)));
}

/** What the pager says above its controls: which slice of what.
 *
 * Counts the whole filtered list rather than the page in hand, which is the
 * figure the seller is deciding against — "1–25 of 143" tells them there is
 * more, and "25 resources" does not. */
export function pageSummary(
	offset: number,
	shown: number,
	total: number,
	noun: string
): string {
	if (total === 0) {
		return `No ${noun}`;
	}
	const first = offset + 1;
	const last = offset + shown;
	return `${first}–${last} of ${total} ${noun}`;
}

/** What a run that could not be read says. A read that failed is not a run
 *  with nothing in it, and the page holds those two apart. */
export const RUN_UNREAD = 'We could not load this import. Anything already running carries on.';

/** What the seller is told while the reading happens somewhere else. One
 *  sentence; which bytes travel is the `your-files` guide's to hold. */
export const READING_HAPPENS_ON_YOUR_COMPUTER =
	'Each resource is opened on your own computer.';
