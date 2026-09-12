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
	ImportRunState,
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

/** The one thing a run is doing, which is what its page is laid out around.
 *
 * Six rather than the server's six states, and they are not the same six: a
 * run in `reading` is either waiting for its shop to be enumerated, waiting
 * for the seller to tick what to bring, or reading what they ticked, and
 * those are three different pages. The three the server tells apart —
 * reviewing, committing, settled — it tells apart already. */
export type RunStage =
	'listing' | 'selecting' | 'reading' | 'reviewing' | 'committing' | 'done' | 'failed';

/** Which step a run is standing on.
 *
 * `read_total` is what splits the first three. It is null until the
 * enumeration lands, and a total of nothing must not read as a shop with
 * nothing in it — which is the same reason the column is nullable. Once it is
 * known, anything still `listed` is a resource the seller has not answered
 * for, so the page is the tick list; with none left the device is reading. */
export function stageFrom(run: ImportRunHead): RunStage {
	switch (run.state) {
		case 'reviewing':
			return 'reviewing';
		case 'committing':
			return 'committing';
		case 'complete':
			return 'done';
		case 'failed':
		case 'abandoned':
			return 'failed';
		case 'reading':
			break;
	}
	if (run.read_total === null) {
		return 'listing';
	}
	return run.counts.listed > 0 ? 'selecting' : 'reading';
}

/** The badge a run state renders as, on its own page and in the listing
 *  alike.
 *
 * Over the state rather than over the stage, because the listing draws a
 * badge from a head and the three `reading` stages are one word to a seller
 * reading a list: the run is under way.
 *
 * `unrecognised` is not decoration: a state added in Rust degrades to saying
 * so rather than rendering an unstyled blank. */
const BADGE: Record<ImportRunState, StageBadge> = {
	reading: { tone: 'run', label: 'Under way' },
	reviewing: { tone: 'warn', label: 'Needs you' },
	committing: { tone: 'run', label: 'Adding to your catalogue' },
	complete: { tone: 'ok', label: 'Imported' },
	failed: { tone: 'bad', label: 'Finished with problems' },
	abandoned: { tone: 'soon', label: 'Abandoned' }
};

export interface StageBadge {
	tone: PillTone;
	label: string;
}

/** `state` is a bare string rather than the closed union on purpose: the
 *  value arrives off the wire, and typing the parameter as the union would
 *  make the unknown arm unreachable in the type checker while staying
 *  perfectly reachable at runtime. */
export function runBadge(state: string): StageBadge {
	return BADGE[state as ImportRunState] ?? { tone: 'soon', label: 'Unknown' };
}

/** What the page leads with, and the line under it. */
export interface StageCopy {
	headline: string;
	detail: string;
}

const STAGE_COPY: Record<RunStage, StageCopy> = {
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
		headline: 'Some of these look like resources you already have.',
		detail:
			'Answer each pair below and nothing is created until you do. Anything you leave for ' +
			'later is not in the way.'
	},
	committing: {
		headline: 'Adding them to your catalogue.',
		detail: 'A batch at a time, so a closed window loses only the batch in flight.'
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

// ------------------------------------------------------------------ progress

/** How many resources this run is still working through.
 *
 * Everything but the skips, because a skip is mostly a resource the seller
 * did not tick: counting those in the denominator would leave the bar stuck
 * short of its own total for ever, reading as an import that stalled. */
export function inPlay(counts: ImportRunCounts): number {
	return (
		counts.listed +
		counts.selected +
		counts.read +
		counts.matched +
		counts.review +
		counts.imported +
		counts.failed
	);
}

/** How many have been read: everything past the read, whatever became of it
 *  afterwards. A resource that was read and then failed was still read. */
export function readSoFar(counts: ImportRunCounts): number {
	return counts.read + counts.matched + counts.review + counts.imported + counts.failed;
}

/** The live bar, in one line.
 *
 * Three figures and no more: how far the reading has got, how many are
 * waiting on the seller, and how many are in the catalogue. The two that are
 * zero drop out rather than reading as claims about nothing — "0 need you"
 * invites a seller to go looking for the question. */
export function progressLine(counts: ImportRunCounts, total: number): string {
	const parts = [`${readSoFar(counts)} of ${total} read`];
	if (counts.review > 0) {
		parts.push(`${counts.review} need you`);
	}
	if (counts.imported > 0) {
		parts.push(`${counts.imported} imported`);
	}
	return parts.join(' · ');
}

/** The line the listing draws under a run's name.
 *
 * A settled run says what it did; a running one says how far it got. Neither
 * invents a total: a run whose enumeration has not landed has no denominator
 * and says so in words rather than in a zero. */
export function countsLine(counts: ImportRunCounts, readTotal: number | null): string {
	if (readTotal === null) {
		return 'Reading what is in your shop.';
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
		case 'listing':
			return 'Nothing has been listed yet.';
		case 'selecting':
		case 'reading':
		case 'reviewing':
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
