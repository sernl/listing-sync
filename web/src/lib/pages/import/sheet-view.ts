// The spreadsheet import's batch page, as a model: which one thing the batch
// is saying, which rows still want bytes, which dropped file belongs to which
// row, and what a commit would create. Pure, so it tests without a DOM.
//
// The distinctions this file exists to hold apart are the ones that cost most
// when they are lost. A batch nobody could read is never a batch with no rows.
// A count nobody read is never a zero. A file that names two rows is never
// placed on one of them by a guess. And a row is named by the seller's own
// spreadsheet row number, never by its index in an array.

import type {
	BatchStateView,
	CommitAck,
	ImportBatchDetailView,
	ImportBatchView,
	ImportProblem,
	ImportRowView,
	ImportWarning,
	RowStateView
} from '$lib/api';
import { formatBytes } from '$lib/authoring';
import type { ReadState } from '$lib/pages/automations/read-state';
import { MARKETPLACE_NAME, SHORT_NAME } from '$lib/platforms';
import type { PillTone } from './import-view';

/** Every batch state, as a runtime list.
 *
 * Built from a total record rather than written out as an array, so a state
 * added to `BatchStateView` fails the type check here instead of quietly
 * leaving the sweep below narrower than the vocabulary it claims to cover. */
const EVERY_BATCH_STATE: Record<BatchStateView, true> = {
	parsed: true,
	attaching: true,
	importing: true,
	imported: true,
	failed: true,
	abandoned: true
};

export const BATCH_STATES = Object.keys(EVERY_BATCH_STATE) as BatchStateView[];

/** Every row state, on the same terms. */
const EVERY_ROW_STATE: Record<RowStateView, true> = {
	parsed: true,
	attached: true,
	creating: true,
	created: true,
	published: true,
	failed: true,
	skipped: true
};

export const ROW_STATES = Object.keys(EVERY_ROW_STATE) as RowStateView[];

// ------------------------------------------------------------------ counting

/** Where the rows stand, as a partition: every row falls in exactly one of
 *  these, so they sum to `total`.
 *
 * `unrecognised` exists so a row carrying a state this console has never heard
 * of is counted rather than discarded onto a junk key while the other figures
 * stop summing. */
export interface RowTally {
	/** Passed the parse, holding no bytes yet. */
	waiting: number;
	/** Holding the bytes a create will name. */
	attached: number;
	/** Claimed by a commit whose create has not run. */
	creating: number;
	/** A resource in the catalogue exists for this row. */
	created: number;
	/** Refused, by the parse or by the create, each carrying its own reason. */
	refused: number;
	/** Deliberately left out. Unreachable today, counted so it cannot be lost. */
	skipped: number;
	/** Carrying a state this console does not know. */
	unrecognised: number;
	total: number;
}

/** Which figure each row state is counted under.
 *
 * `published` counts as created because it is: a published row has a resource
 * in the catalogue and a listing beyond it. This phase mints no job, so no row
 * reaches it, and the arm exists rather than the state being unhandled. */
const TALLY_BUCKET: Record<RowStateView, keyof Omit<RowTally, 'total'>> = {
	parsed: 'waiting',
	attached: 'attached',
	creating: 'creating',
	created: 'created',
	published: 'created',
	failed: 'refused',
	skipped: 'skipped'
};

export function tallyRows(rows: readonly ImportRowView[]): RowTally {
	const counted: RowTally = {
		waiting: 0,
		attached: 0,
		creating: 0,
		created: 0,
		refused: 0,
		skipped: 0,
		unrecognised: 0,
		total: 0
	};
	for (const row of rows) {
		counted[TALLY_BUCKET[row.state] ?? 'unrecognised'] += 1;
		counted.total += 1;
	}
	return counted;
}

/** What pressing import would do, in the two numbers the confirmation states.
 *
 * `unknown` is carried rather than folded into either figure: a row in a state
 * this console does not know is not known to be created and not known to be
 * left alone, and putting it in one of them would make that figure a claim
 * nothing supports. */
export interface Preview {
	/** Rows that will become resources. */
	create: number;
	/** Rows the parse refused, which a commit leaves alone. */
	leave: number;
	/** Rows a commit has already created. */
	done: number;
	unknown: number;
}

export function previewOf(tally: RowTally): Preview {
	return {
		create: tally.waiting + tally.attached + tally.creating,
		leave: tally.refused + tally.skipped,
		done: tally.created,
		unknown: tally.unrecognised
	};
}

/** What the confirmation says, from the counts it read.
 *
 * One sentence rather than two figures a template joins, because the second
 * clause is dropped entirely when nothing was refused: "and 0 rows will be
 * left alone" reads as a fault the seller then goes looking for. */
export function previewSentence(preview: Preview): string {
	const resources = preview.create === 1 ? '1 resource' : `${preview.create} resources`;
	const head = `${resources} will be created in your catalogue.`;
	if (preview.leave === 0) {
		return head;
	}
	const rows = preview.leave === 1 ? '1 row was' : `${preview.leave} rows were`;
	return `${head} ${rows} refused when the sheet was read, and will be left alone.`;
}

// -------------------------------------------------------------------- gating

/** Whether this row must hold bytes before a commit will run.
 *
 * D32 generalises the founder's live-only rule to every marketplace row: a row
 * on a marketplace tab needs a file whether it asks for draft or live, and a
 * Teachouse row needs none. A row the parse refused needs none either, because
 * it will never be created. */
export function needsFile(row: ImportRowView): boolean {
	return row.inventory !== null && row.state === 'parsed' && !row.file_attached;
}

export function awaitingRows(rows: readonly ImportRowView[]): ImportRowView[] {
	return rows.filter(needsFile);
}

export function attachedRows(rows: readonly ImportRowView[]): ImportRowView[] {
	return rows.filter((row) => row.file_attached);
}

// --------------------------------------------------------------------- stage

/** The one thing a batch is saying, which is what the page leads with.
 *
 * `unrecognised` is not decoration: a state added in Rust degrades to saying
 * so, with the server's own word, rather than blanking the page. */
export type SheetStage =
	| { kind: 'parsed'; tally: RowTally; preview: Preview; awaiting: number }
	| { kind: 'attaching'; tally: RowTally; preview: Preview; awaiting: number }
	| { kind: 'importing'; tally: RowTally }
	| { kind: 'imported'; tally: RowTally }
	| { kind: 'failed'; tally: RowTally; detail: string | null }
	| { kind: 'abandoned'; tally: RowTally }
	| { kind: 'unrecognised'; state: string };

export function stageOf(detail: ImportBatchDetailView): SheetStage {
	return stageFrom(detail.state, detail.failure_detail, detail.rows);
}

/** The stage from the two facts that decide it: the state the server holds,
 *  and where its rows stand.
 *
 * `state` is a bare string rather than `BatchStateView` on purpose. The value
 * arrives off the wire, and typing the parameter as the closed union would
 * make the unknown arm unreachable in the type checker while staying perfectly
 * reachable at runtime. */
export function stageFrom(
	state: string,
	failureDetail: string | null,
	rows: readonly ImportRowView[]
): SheetStage {
	const tally = tallyRows(rows);
	switch (state) {
		case 'parsed':
			return {
				kind: 'parsed',
				tally,
				preview: previewOf(tally),
				awaiting: awaitingRows(rows).length
			};
		case 'attaching':
			return {
				kind: 'attaching',
				tally,
				preview: previewOf(tally),
				awaiting: awaitingRows(rows).length
			};
		case 'importing':
			return { kind: 'importing', tally };
		case 'imported':
			return { kind: 'imported', tally };
		case 'failed':
			return { kind: 'failed', tally, detail: failureDetail };
		case 'abandoned':
			return { kind: 'abandoned', tally };
		default:
			return { kind: 'unrecognised', state };
	}
}

/** The badge and the sentence one stage renders as. */
export interface StageCopy extends StageBadge {
	line: string;
}

/** The badge one batch state renders as, in the listing and on the page alike.
 *
 * Apart from [`presentStage`] because the listing draws a badge and no
 * sentence: a row that computed a sentence about rows it never read would be
 * discarding a claim rather than not making one. */
const BADGE: Record<BatchStateView, StageBadge> = {
	parsed: { tone: 'run', label: 'Read' },
	attaching: { tone: 'run', label: 'Adding files' },
	importing: { tone: 'run', label: 'Importing' },
	imported: { tone: 'ok', label: 'Imported' },
	failed: { tone: 'bad', label: 'Finished with problems' },
	abandoned: { tone: 'soon', label: 'Abandoned' }
};

export interface StageBadge {
	tone: PillTone;
	label: string;
}

export function badgeOf(state: string): StageBadge {
	return BADGE[state as BatchStateView] ?? { tone: 'soon', label: 'Unknown' };
}

export function presentStage(stage: SheetStage): StageCopy {
	switch (stage.kind) {
		case 'parsed':
			return {
				...BADGE.parsed,
				line:
					stage.awaiting === 0
						? 'Your sheet has been read. Check the report below, then import it.'
						: 'Your sheet has been read. Check the report below, then add the files it names.'
			};
		case 'attaching':
			return {
				...BADGE.attaching,
				line:
					stage.awaiting === 0
						? 'Every row that names a marketplace has its file. You can import now.'
						: `${stage.awaiting} ${stage.awaiting === 1 ? 'row is' : 'rows are'} still waiting for a file.`
			};
		case 'importing':
			return {
				...BADGE.importing,
				line: 'Creating your resources, a batch at a time. You can leave this page.'
			};
		case 'imported':
			return { ...BADGE.imported, line: NOTHING_SENT };
		case 'failed':
			return {
				...BADGE.failed,
				line: stage.detail ?? 'Some rows did not import. Each one says why below.'
			};
		case 'abandoned':
			return {
				...BADGE.abandoned,
				line: 'You gave this import up. Nothing from it was created.'
			};
		case 'unrecognised':
			return {
				...badgeOf(stage.state),
				line: `This import is in a state this page does not know: ${stage.state}. Nothing has been changed.`
			};
	}
}

/** The sentence the finished page carries, because the seller's next question
 *  is whether their shops have changed. They have not: D1 keeps every
 *  marketplace request on the seller's own device, and this phase mints no job
 *  at all. */
export const NOTHING_SENT =
	'Nothing has been sent to a marketplace. These resources are in your catalogue here; ' +
	'publishing them runs later, from your own device.';

// -------------------------------------------------------------------- report

/** How one row renders in the report. */
export interface ReportRow {
	key: string;
	sheet: string;
	/** The seller's own spreadsheet row number. */
	ordinal: number;
	/** How the report addresses this row, in the terms the seller reads in
	 *  their own margin. */
	label: string;
	/** The title, where a server sends one, else the filename the row named,
	 *  else nothing. Never an invented stand-in. */
	name: string | null;
	fileName: string | null;
	marketplace: string | null;
	state: RowStateView;
	tone: PillTone;
	stateLabel: string;
	problems: ImportProblem[];
	failureDetail: string | null;
	attached: boolean;
	/** The resource this row created, where it named one. Null everywhere
	 *  else: a link this page cannot address is not offered. */
	href: string | null;
}

const ROW_TONE: Record<RowStateView, PillTone> = {
	parsed: 'soon',
	attached: 'run',
	creating: 'run',
	created: 'ok',
	published: 'ok',
	failed: 'bad',
	skipped: 'soon'
};

const ROW_LABEL: Record<RowStateView, string> = {
	parsed: 'Ready',
	attached: 'File added',
	creating: 'Creating',
	created: 'Created',
	published: 'Published',
	failed: 'Refused',
	skipped: 'Left out'
};

/** How the report addresses one row: its tab and the number in the margin.
 *
 * The ordinal is the seller's own spreadsheet row number and never an index
 * into this array, which is the off-by-one this feature would otherwise
 * ship. */
export function rowLabel(row: { sheet: string; ordinal: number }): string {
	return `${row.sheet}, row ${row.ordinal}`;
}

/** The marketplace worth naming beside a row, or null where the tab already
 *  named it.
 *
 * A grid tab is titled after the marketplace it authors for, so naming both
 * renders "TES GB, row 2 — TES GB". The suffix earns its place only on a row
 * whose tab title and marketplace have come apart, which is what a renamed tab
 * or a registry change produces. */
function marketplaceBeside(row: ImportRowView): string | null {
	if (row.inventory === null) {
		return null;
	}
	const named = SHORT_NAME[row.inventory];
	return named === row.sheet ? null : named;
}

export function reportRows(rows: readonly ImportRowView[]): ReportRow[] {
	return rows.map((row) => ({
		key: `${row.sheet}:${row.ordinal}`,
		sheet: row.sheet,
		ordinal: row.ordinal,
		label: rowLabel(row),
		name: row.title ?? row.file_name,
		fileName: row.file_name,
		marketplace: marketplaceBeside(row),
		state: row.state,
		tone: ROW_TONE[row.state] ?? 'soon',
		stateLabel: ROW_LABEL[row.state] ?? row.state,
		problems: row.problems,
		failureDetail: row.failure_detail,
		attached: row.file_attached,
		href: resourceHref(row)
	}));
}

/** The resource one created row links to, where the row names one.
 *
 * Two conditions, because the identifier arrives before the resource does. A
 * commit reserves `product` when it claims the row, so a row still `creating`
 * names an identifier no resource answers to yet and a link to it would open a
 * page that is not there; only `created` and `published` have one. The
 * identifier is then read as a presence test rather than against the two values
 * the wire declares, because a body that omits the key entirely would otherwise
 * link to the word `undefined`. */
export function resourceHref(row: ImportRowView): string | null {
	if (row.state !== 'created' && row.state !== 'published') {
		return null;
	}
	return row.product ? `/resources/${row.product}` : null;
}

// ------------------------------------------------------------------ warnings

export function warningLine(warning: ImportWarning): string {
	switch (warning.kind) {
		case 'new_label': {
			const rows = warning.rows === 1 ? '1 row' : `${warning.rows} rows`;
			return `“${warning.name}” is a new label, on ${rows}. It will be created.`;
		}
		case 'no_connection': {
			const rows = warning.rows === 1 ? '1 row asks' : `${warning.rows} rows ask`;
			return `${MARKETPLACE_NAME[warning.marketplace]} is not connected, and ${rows} to go live there. They will be created here and can be published once it is connected.`;
		}
	}
}

// ------------------------------------------------------------------ matching

/** One row still waiting for its bytes, as the matcher sees it. */
export interface WaitingRow {
	sheet: string;
	ordinal: number;
	/** The filename the seller declared in the `File` column. A row that
	 *  declared none can never be matched by name and is placed by hand. */
	fileName: string | null;
}

export interface RowRef {
	sheet: string;
	ordinal: number;
}

/** Which of the three passes placed a file. Carried so the panel can say how
 *  a file was placed, and so a looser match is visible rather than silent. */
export type MatchPass = 'exact' | 'case' | 'stem';

export type FileMatch =
	| { name: string; kind: 'matched'; row: RowRef; pass: MatchPass }
	| { name: string; kind: 'unplaced'; reason: string };

export const NO_ROW_NAMED_IT = 'No row named this file. Place it by hand, or leave it out.';

export const SEVERAL_ROWS_NAMED_IT =
	'More than one row names this file, so it was not placed for you. Place it by hand.';

export const ROW_ALREADY_TAKEN =
	'Another file in this drop was already placed on the row this one names.';

/** What the passes compare. Exact first, then case, then the name without its
 *  extension: each is looser than the one before, and the first that finds
 *  anything at all decides — including when what it finds is two rows. */
const PASSES: readonly { pass: MatchPass; key: (name: string) => string }[] = [
	{ pass: 'exact', key: (name) => name },
	{ pass: 'case', key: (name) => name.toLowerCase() },
	{ pass: 'stem', key: (name) => stemOf(name).toLowerCase() }
];

/** A filename without its last extension.
 *
 * A name that is all extension — a leading dot and nothing before it — keeps
 * its whole self, because an empty stem would match every other such name and
 * turn the third pass into an ambiguity generator. */
export function stemOf(name: string): string {
	const dot = name.lastIndexOf('.');
	return dot > 0 ? name.slice(0, dot) : name;
}

function refOf(row: WaitingRow): RowRef {
	return { sheet: row.sheet, ordinal: row.ordinal };
}

function keyOf(row: RowRef): string {
	return `${row.sheet}:${row.ordinal}`;
}

/** Place each dropped file on the one row that named it, or on none.
 *
 * Three passes, tightest first, and a pass that finds more than one row stops
 * there rather than falling through to a looser one: a name that is already
 * ambiguous exactly does not become less so when compared more loosely. A file
 * matching no row and a file matching several both go to the strip, with
 * different reasons, and neither is ever guessed at.
 *
 * A row is claimed by at most one file in a drop. Two files that name one row
 * would otherwise both be bound, the second silently replacing the first. */
export function matchFiles(
	names: readonly string[],
	waiting: readonly WaitingRow[]
): FileMatch[] {
	const taken = new Set<string>();
	return names.map((name) => {
		for (const { pass, key } of PASSES) {
			const wanted = key(name);
			if (wanted === '') {
				continue;
			}
			const found = waiting.filter(
				(row) => row.fileName !== null && key(row.fileName) === wanted
			);
			if (found.length === 0) {
				continue;
			}
			if (found.length > 1) {
				return { name, kind: 'unplaced', reason: SEVERAL_ROWS_NAMED_IT };
			}
			const row = refOf(found[0]);
			if (taken.has(keyOf(row))) {
				return { name, kind: 'unplaced', reason: ROW_ALREADY_TAKEN };
			}
			taken.add(keyOf(row));
			return { name, kind: 'matched', row, pass };
		}
		return { name, kind: 'unplaced', reason: NO_ROW_NAMED_IT };
	});
}

// ------------------------------------------------------------------ headroom

/** This organisation's storage after an upload, in the words `UploadedView`
 *  answers with. Null until an upload has actually reported them, because a
 *  headroom nobody read is not a headroom of zero. */
export function headroomLine(
	uploaded: { stored_bytes: number; storage_bytes_max: number } | null
): string | null {
	if (uploaded === null) {
		return null;
	}
	return `${formatBytes(uploaded.stored_bytes)} of ${formatBytes(uploaded.storage_bytes_max)} stored`;
}

// ------------------------------------------------------------------ progress

/** How far a commit has got.
 *
 * `fraction` is null where the total is zero: a bar drawn from a division that
 * has no answer is a number this page did not read. */
export type CommitProgress =
	| { kind: 'unstarted' }
	| { kind: 'running'; settled: number; total: number; fraction: number | null }
	| { kind: 'complete'; settled: number; total: number };

/** Rows a commit has finished with, either way. */
export function settledCount(tally: RowTally): number {
	return tally.created + tally.refused + tally.skipped;
}

/** Progress as the acknowledgement stated it, from the two figures the server
 *  answers for exactly this.
 *
 * `remaining` and `total` rather than a sum of chunks. `CommitAck`'s `applied`,
 * `skipped` and `failed` are one chunk's figures, following `ImportAck`, whose
 * own doc calls them what "this page added", and a client that added them up
 * would be keeping a second ledger of something the server already counts —
 * one a seller who reloads the page mid-import arrives with nothing in, so the
 * bar would restart at zero and lie about work already done. `total` is every
 * row the parse accepted and does not move as rows are created.
 *
 * Whether the run is over is read off `remaining` rather than off `complete`,
 * because the server defines the one as the other: a chunk that left rows
 * outstanding is not finished however it labelled itself. */
export function progressFrom(ack: CommitAck | null): CommitProgress {
	if (ack === null) {
		return { kind: 'unstarted' };
	}
	const settled = ack.total - ack.remaining;
	if (ack.remaining === 0) {
		return { kind: 'complete', settled, total: ack.total };
	}
	return {
		kind: 'running',
		settled,
		total: ack.total,
		fraction: ack.total > 0 ? settled / ack.total : null
	};
}

/** What the seller is told when chunk after chunk leaves the same rows to do.
 *
 * The loop stops there rather than asking again for ever: a chunk that left
 * `remaining` where it found it and did not report the batch complete is a
 * server that cannot be helped by being asked a second time. */
export const COMMIT_STALLED =
	'The import stopped making progress. Nothing further was created; try again in a moment.';

export function progressLine(progress: CommitProgress): string {
	switch (progress.kind) {
		case 'unstarted':
			return 'Starting…';
		case 'running':
			return `${progress.settled} of ${progress.total} rows done.`;
		case 'complete':
			return `All ${progress.total} rows done.`;
	}
}

// ------------------------------------------------------- the listing's panel

/** What the Import page's spreadsheet panel says in place of its rows, and
 *  null where it has rows to draw.
 *
 * The failed wording states what did and did not happen, because a seller told
 * they have no imports when the read simply failed will believe it. */
export function listCopy<Row>(state: ReadState<Row>): { title: string; body: string } | null {
	switch (state.kind) {
		case 'pending':
			return { title: 'Reading your spreadsheet imports…', body: '' };
		case 'failed':
			return {
				title: 'Your spreadsheet imports could not be read',
				body:
					'This panel cannot say which imports you have, so it is showing none. ' +
					'Nothing has been changed.'
			};
		case 'empty':
			return {
				title: 'No spreadsheet import yet',
				body: 'Download the template, fill it in, and upload it here.'
			};
		case 'rows':
			return null;
	}
}

/** How one batch reads in the listing. */
export interface BatchRow {
	id: string;
	href: string;
	name: string;
	line: string;
	tone: PillTone;
	label: string;
	createdAt: number;
	open: boolean;
}

const OPEN_STATES: Record<BatchStateView, boolean> = {
	parsed: true,
	attaching: true,
	importing: true,
	imported: false,
	failed: false,
	abandoned: false
};

export function isOpen(state: string): boolean {
	return OPEN_STATES[state as BatchStateView] === true;
}

export function batchRows(batches: readonly ImportBatchView[]): BatchRow[] {
	return batches.map((batch) => {
		const badge = badgeOf(batch.state);
		const rows = batch.row_count === 1 ? '1 row' : `${batch.row_count} rows`;
		const refused =
			batch.failed_count === 0 ? '' : `, ${batch.failed_count} refused when it was read`;
		return {
			id: batch.id,
			href: batchHref(batch.id),
			name: batch.source_name,
			line: `${rows}${refused}`,
			tone: badge.tone,
			label: badge.label,
			createdAt: batch.created_at,
			open: isOpen(batch.state)
		};
	});
}

export function batchHref(batch: string): string {
	return `/imports/${batch}`;
}

/** Why the upload control is unavailable while a batch is open.
 *
 * A stated reason rather than an absent control: a card that simply loses its
 * button reads as a fault, and the one thing the seller can do about this is
 * reach the batch that is in the way. */
export const IMPORT_ALREADY_OPEN =
	'You already have an import open. Finish it or give it up before starting another.';

/** What the seller is told when a read or a write came back with nothing to
 *  say for itself. */
export const REFUSED_WITHOUT_REASON =
	'That was refused and no reason came back. Try again in a moment.';

export const BATCH_UNREAD =
	'This import could not be read, so this page is showing none of it. ' +
	'Nothing has been changed.';

/** The batch state a refusal carried, where it carried one this console knows.
 *
 * The body is `unknown` because it is the server's own `detail` field, which no
 * type on this side constrains. */
function refusedState(detail: unknown): BatchStateView | null {
	if (typeof detail !== 'object' || detail === null) {
		return null;
	}
	const state = (detail as Record<string, unknown>).batch_state;
	return typeof state === 'string' && BATCH_STATES.includes(state as BatchStateView)
		? (state as BatchStateView)
		: null;
}

/** Why a bind or an unbind was refused 409, read from the state the refusal
 *  carried rather than assumed from the status alone.
 *
 * The two closed cases differ in what the seller does next, which is the whole
 * reason the server sends the state: an importing batch is being created right
 * now and is waited on, while a settled one is only read. Null where the state
 * is missing or is one that does take files, so the caller states the server's
 * own sentence rather than this page inventing a reason for a refusal it cannot
 * account for. */
export function batchClosedSay(detail: unknown): string | null {
	const state = refusedState(detail);
	if (state === null) {
		return null;
	}
	switch (state) {
		case 'importing':
			return 'This import is being created right now, so no more files can be added. Reload the page to watch it finish.';
		case 'imported':
		case 'failed':
		case 'abandoned':
			return 'This import has been finished or given up, so no more files can be added.';
		case 'parsed':
		case 'attaching':
			return null;
	}
}
/** The rows a commit refused for want of bytes, as the server named them.
 *
 * D32's gate answers its own count and the first of the rows it is waiting on,
 * so the page can name them rather than sending the seller back to read the
 * whole report. Read off the refusal rather than recomputed from the rows this
 * page holds: the server decided, and a page that worked the list out again
 * could name a different set than the one that caused the refusal. */
export interface AwaitingFiles {
	/** Every row still waiting, which is not the length of `rows`: the server
	 *  names only the first of them. */
	count: number;
	rows: RowRef[];
}

/** What a 409 said about rows still holding no file, or null where it said
 *  nothing of the kind — a settled batch's refusal carries a state instead —
 *  so the caller falls through to the server's own sentence rather than
 *  inventing a reason for a refusal it cannot account for. */
export function awaitingFrom(detail: unknown): AwaitingFiles | null {
	if (typeof detail !== 'object' || detail === null) {
		return null;
	}
	const count = (detail as Record<string, unknown>).awaiting;
	if (typeof count !== 'number' || !Number.isFinite(count)) {
		return null;
	}
	return { count, rows: namedRows((detail as Record<string, unknown>).rows) };
}

/** The rows a refusal named, keeping only those it named completely.
 *
 * A row missing its tab or its number cannot be addressed in the terms the
 * seller reads, and one filled in from a default would send them to the wrong
 * margin. */
function namedRows(raw: unknown): RowRef[] {
	if (!Array.isArray(raw)) {
		return [];
	}
	const named: RowRef[] = [];
	for (const entry of raw) {
		if (typeof entry !== 'object' || entry === null) {
			continue;
		}
		const { sheet, ordinal } = entry as Record<string, unknown>;
		if (typeof sheet === 'string' && typeof ordinal === 'number' && Number.isFinite(ordinal)) {
			named.push({ sheet, ordinal });
		}
	}
	return named;
}

/** The banner that refusal renders under. A category rather than a count, so
 *  one row and forty read the same. */
export const AWAITING_TITLE = 'Files still to add';

/** What the seller reads when the commit was refused for want of bytes.
 *
 * The rows are named in the page's own terms — the tab, and the number in the
 * seller's own margin — because that is where they go to fix it. The server
 * sends the first of them only, so a count larger than the list says how many
 * more there are rather than letting the list read as the whole of it. */
export function awaitingSay(awaiting: AwaitingFiles): string {
	const head =
		awaiting.count === 1
			? 'One row still needs its file before this import can run'
			: `${awaiting.count} rows still need their files before this import can run`;
	if (awaiting.rows.length === 0) {
		return `${head}.`;
	}
	const named = awaiting.rows.map(rowLabel).join('; ');
	const rest = awaiting.count - awaiting.rows.length;
	return rest > 0 ? `${head}: ${named}, and ${rest} more.` : `${head}: ${named}.`;
}
