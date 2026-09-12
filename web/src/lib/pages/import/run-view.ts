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
	ImportRunView,
	ObservedPrice,
	ReviewPairView,
	ReviewSideView
} from '$lib/api';
import type { InventoryId, Marketplace } from '$lib/generated/vocab';
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
	reading: { tone: 'run', label: 'Reading resources' },
	reviewing: { tone: 'warn', label: 'Needs you' },
	confirming: { tone: 'warn', label: 'Ready for your confirmation' },
	committing: { tone: 'run', label: 'Adding to your catalogue' },
	interrupted: { tone: 'warn', label: 'Reading paused' },
	done: { tone: 'ok', label: 'Imported' },
	failed: { tone: 'bad', label: 'Stopped' }
};

export function runBadge(run: ImportRunHead): StageBadge {
	if (run.state === 'complete' && (run.counts.failed > 0 || run.counts.skipped > 0)) {
		return { tone: 'warn', label: 'Finished with items left out' };
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
		detail: 'Open the app on a connected device to begin reading. No device is reading this shop yet.'
	},
	interrupted: {
		headline: 'Reading has paused.',
		detail: 'Reconnect on the device that was reading, then resume. Resources already added stay in your catalogue.'
	},
	confirming: {
		headline: 'Ready to add to your catalogue.',
		detail: 'Check the results below, then confirm. Nothing is published to a marketplace by importing.'
	},
	listing: {
		headline: 'Reading what is in your shop.',
		detail:
			'Your computer is listing every resource it can see. Nothing is copied yet, and you ' +
			'choose what to bring across next.'
	},
	selecting: {
		headline: 'Choose what to bring across.',
		detail: 'Tick the resources you want in Resources. Anything you leave is not read at all.'
	},
	reading: {
		headline: 'Reading the resources you chose.',
		detail:
			'Your computer opens each one and sends us what describes it. You can leave this page: ' +
			'the reading carries on and this list fills as it goes.'
	},
	reviewing: {
		headline: 'Check these resources before adding them.',
		detail: 'Answer any duplicate questions below, then confirm what is ready.'
	},
	committing: {
		headline: 'Adding them to your catalogue.',
		detail: 'The server is adding the resources you approved. You can close this page.'
	},
	done: {
		headline: 'This import is finished.',
		detail: 'Everything below is in Resources, apart from what the list says was left out.'
	},
	failed: {
		headline: 'This import did not finish.',
		detail: 'What it did bring across is in Resources. Each resource below says what happened.'
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
		return 'No resource count yet.';
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
		name: item.title ?? item.locator,
		price: item.price === null ? null : formatPrice({ Paid: item.price }),
		coverUrl: item.cover_url,
		label: badge.label,
		tone: badge.tone,
		reason: item.skip_reason ?? item.failure_detail ?? '',
		product: item.product_id
	};
}

export function itemRows(run: ImportRunView): ItemRow[] {
	return run.items.map(rowOf);
}

/** The resources the seller is being asked to tick, in the order the shop
 *  listed them. The same rows the list below draws, so one resource reads
 *  the same in both places. */
export function selectionRows(run: ImportRunView): ItemRow[] {
	return run.items.filter((item) => item.state === 'listed').map(rowOf);
}

/** Why the Continue control cannot be pressed, or null where it can.
 *
 * A disabled control states its reason, which is what `Button` requires of
 * every caller; and the reason here is a thing the seller does rather than a
 * thing that is wrong, so it reads as an instruction. */
export function selectionBlocked(chosen: number, sending: boolean): string | null {
	if (sending) {
		return 'Your choice is being sent.';
	}
	return chosen === 0 ? 'Tick at least one resource to bring across.' : null;
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
const JUST_READ = 'Just read from your shop';

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
	'One of these stays and the other goes. Choose which one to keep, then which wording it ' +
	'keeps for each field.';

/** How long a merge can be undone for, said as the deadline the seller was
 *  given rather than as a policy. */
export const MERGE_IS_REVERSIBLE =
	'You can undo a merge for thirty days. Until then both listings stay linked and nothing is ' +
	'deleted from your marketplaces.';

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
			return 'Waiting for a device to find the first resource.';
		case 'interrupted':
			return 'No resources were recorded before reading paused.';
		case 'listing':
			return 'Nothing has been listed yet.';
		case 'selecting':
		case 'reading':
		case 'reviewing':
		case 'confirming':
		case 'committing':
			return 'Nothing has arrived yet.';
		case 'done':
			return 'Your shop had nothing in it to bring across.';
		case 'failed':
			return 'Nothing was recorded before this import stopped.';
	}
}

/** What a run that could not be read says. A read that failed is not a run
 *  with nothing in it, and the page holds those two apart. */
export const RUN_UNREAD = 'We could not read this import. Anything already running carries on.';

/** What the seller is told while the reading happens somewhere else. */
export const READING_HAPPENS_ON_YOUR_COMPUTER =
	'Each resource is opened on your own computer. Only what describes it — its details and its ' +
	'thumbnail — is sent here.';
