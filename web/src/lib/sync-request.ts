// A migrate request's page, as a model: which one thing the request is saying,
// what that says to the seller, and the two facts that render beside it. Pure,
// so it tests without a component.
//
// The distinctions this file exists to hold apart are the ones that are
// cheapest to lose and most expensive to lose. An empty result is never a
// failure and a failure is never an empty result, because a seller told their
// shop was empty when the read actually broke will believe it. A coverage
// figure nobody took is never a zero. And a device's reason for skipping a
// listing is rendered in the device's words rather than in ours.

import type {
	ConnectionView,
	SyncRequestHead,
	SyncCoverageView,
	SyncRequestBody,
	SyncRequestView,
	SyncResourceState,
	SyncResourceView,
	SyncTermCoverage
} from '$lib/api';
import { TRANSPORT_OF, onSellerDevice } from '$lib/inventory';
import { MARKETPLACE_OF } from '$lib/listings-view';
import { PLATFORMS, SHORT_NAME } from '$lib/platforms';
import type { InventoryId, Marketplace } from '$lib/generated/vocab';

/** The tones the console's pills already carry. */
export type StageTone = 'ok' | 'mut' | 'run' | 'bad';

/** Where the resources stand, as a partition: every resource falls in exactly
 *  one of the four, so they sum to `total`. `unsettled` exists so that a
 *  resource still pending on a finished request is counted somewhere rather
 *  than quietly dropped out of the arithmetic, and `unrecognised` so that a
 *  state this console has never heard of is counted rather than discarded onto
 *  a junk key while the other figures stop summing. */
export interface ResourceTally {
	/** Read and canonicalised into the catalogue. */
	imported: number;
	/** Refused, by the device or by the server, each carrying its own reason. */
	skipped: number;
	/** Named and not yet settled either way. */
	unsettled: number;
	/** Carrying a state this console does not know. */
	unrecognised: number;
	total: number;
}

/** The resource states this console knows, and which figure each is counted
 *  under.
 *
 * Keeping this in step with `sync_request_resource_state` in migration 0025 is
 * a manual discipline, not a checked one: the server spells the field as a
 * bare `String` (`jobs.rs`), so there is no Rust enum for typegen to mirror and
 * nothing in the web lane fails when the database's set grows. That is why
 * every read below has an explicit unknown arm — the guarantee is a habit, and
 * habits lapse. */
const TALLY_BUCKET: Record<SyncResourceState, keyof Omit<ResourceTally, 'total'>> = {
	canonicalised: 'imported',
	failed: 'skipped',
	pending: 'unsettled'
};

export function tally(resources: readonly SyncResourceView[]): ResourceTally {
	const counted: ResourceTally = {
		imported: 0,
		skipped: 0,
		unsettled: 0,
		unrecognised: 0,
		total: 0
	};
	for (const resource of resources) {
		counted[TALLY_BUCKET[resource.state] ?? 'unrecognised'] += 1;
		counted.total += 1;
	}
	return counted;
}

/** The one thing a request is saying, which is what the page leads with.
 *
 * Six rather than the four wire states, because two of the states say two
 * different things depending on whether the request names any resource yet,
 * and those pairs are precisely the ones that must not render alike. */
export type SyncStage =
	| { kind: 'waiting_for_device' }
	| { kind: 'queued'; tally: ResourceTally }
	| { kind: 'importing'; tally: ResourceTally }
	| { kind: 'nothing_to_import' }
	| { kind: 'imported'; tally: ResourceTally }
	| { kind: 'failed'; detail: string | null }
	/** A state this console does not know. It degrades to saying so, with the
	 *  server's own word, rather than throwing and blanking the page. */
	| { kind: 'unrecognised'; state: string };

/** What a request is saying, from the two facts that decide it: the state the
 *  server holds, and whether it names any resource yet.
 *
 * `pending` with nothing named is a migrate whose source runs on the seller's
 * own device, which is the only shape the server accepts with no resources;
 * `pending` with resources named is an ordinary sync that has not started.
 * `enqueued` with nothing named is a shop that was read and held nothing, and
 * it is a completed import rather than a failure — the whole reason this
 * function exists rather than a template reading `state` directly. */
export function stageOf(view: SyncRequestView): SyncStage {
	return stageFrom(view.state, view.failure_detail, tally(view.resources));
}

/** The stage of one row of the request list, which carries counts rather than
 *  rows.
 *
 * `imported` is derived as total minus failed, which is exact where it is
 * read: only the `imported` stage renders it, and that stage is reached only
 * from `enqueued`, where every resource has settled. On a draining request the
 * page shows the arrived count, which is `total` and is exact. */
export function headStage(head: SyncRequestHead): SyncStage {
	return stageFrom(head.state, null, {
		imported: Math.max(head.resources_total - head.resources_failed, 0),
		skipped: head.resources_failed,
		unsettled: 0,
		unrecognised: 0,
		total: head.resources_total
	});
}

function stageFrom(
	state: string,
	failureDetail: string | null,
	counted: ResourceTally
): SyncStage {
	switch (state) {
		case 'failed':
			return { kind: 'failed', detail: failureDetail };
		case 'pending':
			return counted.total === 0
				? { kind: 'waiting_for_device' }
				: { kind: 'queued', tally: counted };
		case 'draining':
			return { kind: 'importing', tally: counted };
		case 'enqueued':
			return counted.total === 0
				? { kind: 'nothing_to_import' }
				: { kind: 'imported', tally: counted };
		// Reached when the database's state set grows and this file does not.
		// Degrading here is the difference between a page that says it does not
		// recognise a state and a page that throws on `undefined.kind` inside
		// the template and renders nothing at all.
		default:
			return { kind: 'unrecognised', state };
	}
}

/** What the page says while no page from the device has arrived.
 *
 * One constant because it is the sentence that tells the seller where the work
 * actually happens, and it is true of every surface that cannot start it: a
 * browser, and the application on a machine holding no session for the source.
 * Nothing discovers a request on its own — the import is started from this
 * page, on the computer that holds the session — so the sentence names the
 * action rather than describing a wait. */
export const WAITING_FOR_DEVICE =
	'This import runs on the computer where you are signed in to Tes. ' +
	'Open the Teachouse app there and start it from this page.';

/** The same wait, said on the list rather than on the request's own page.
 *
 * A second constant instead of reusing the one above, because that sentence
 * ends "start it from this page" and is true only on the page that carries the
 * start action. On the list it would tell a seller already standing in the
 * application, on the page they are reading, to do a thing that page cannot
 * do — the only button there creates a further import. So the row points at
 * the request instead, which is where starting actually lives. */
export const WAITING_IN_LIST =
	'Waiting for your device. Open this import to start it on the computer ' +
	'where you are signed in to Tes.';

/** The one line a list row shows for a request.
 *
 * The stage's own headline everywhere except the wait, which is the one stage
 * whose sentence is addressed to a particular page. */
export function listRowLine(stage: SyncStage): string {
	return stage.kind === 'waiting_for_device'
		? WAITING_IN_LIST
		: presentStage(stage).headline;
}

/** Whether this request is one a computer can still be asked to run.
 *
 * Read off the stage rather than re-tested, so the button appears under
 * exactly the state the page calls waiting and the two can never disagree: a
 * request that is draining is already being run by some machine, and one that
 * has enqueued or failed is finished with.
 *
 * False as well while a device version is owed. Offering to run the work on a
 * machine the server has just said is too old to run it invites the seller to
 * press a button whose only possible answer is a refusal, and they would read
 * that refusal as considered rather than as foreseen. */
export function canStartHere(view: SyncRequestView): boolean {
	return stageOf(view).kind === 'waiting_for_device' && deviceUpdateNotice(view) === null;
}

export interface StagePresentation {
	/** The pill's word. */
	label: string;
	/** The sentence the page leads with. */
	headline: string;
	/** What follows it. Empty where the headline is the whole statement. */
	detail: string;
	tone: StageTone;
}

function listings(count: number): string {
	return count === 1 ? '1 listing' : `${count} listings`;
}

export function presentStage(stage: SyncStage): StagePresentation {
	switch (stage.kind) {
		case 'waiting_for_device':
			return {
				label: 'Waiting',
				headline: WAITING_FOR_DEVICE,
				detail: '',
				tone: 'mut'
			};
		case 'queued':
			return {
				label: 'Queued',
				headline: `${listings(stage.tally.total)} named.`,
				detail: 'Nothing has started yet.',
				tone: 'mut'
			};
		case 'importing':
			return {
				label: 'Importing',
				headline: `Importing, ${stage.tally.total} so far.`,
				detail: 'Your device is reading your shop and sending what it finds.',
				tone: 'run'
			};
		case 'nothing_to_import':
			return {
				label: 'Nothing to import',
				headline: 'Your shop had nothing to import.',
				// Says what the console was told, not what is true of the shop.
				// The 2026-08-28 live run saw both Tes dashboard routes answer an
				// empty array with HTTP 200 on an authenticated session, and the
				// founder probe on that is still open, so a claim that the shop
				// held no listings would be exactly the claim that run makes
				// unsafe. The device is the one that read; we report its report.
				detail: 'Your device completed its pass and reported no listings.',
				tone: 'ok'
			};
		case 'imported':
			// An import where nothing landed is not an import, and the list row
			// reads the headline alone, so the fact has to be in the headline
			// rather than in the detail beneath it. Left as the general case it
			// read as a running amber "Imported" over "0 listings imported.",
			// which is a label claiming the opposite of what happened and a
			// figure with no account of itself.
			return stage.tally.imported === 0 && stage.tally.total > 0
				? {
						label: 'Nothing imported',
						headline: nothingImportedHeadline(stage.tally),
						detail:
							stage.tally.skipped > 0
								? 'Open this import to see what was recorded against each one.'
								: 'Nothing was recorded against them.',
						tone: 'run'
					}
				: {
						label: 'Imported',
						headline: `${listings(stage.tally.imported)} imported.`,
						detail: skippedSentence(stage.tally),
						tone: settledCleanly(stage.tally) ? 'ok' : 'run'
					};
		case 'failed':
			return {
				label: 'Failed',
				headline: 'The import failed.',
				detail: stage.detail ?? 'No reason was recorded against the request.',
				tone: 'bad'
			};
		case 'unrecognised':
			return {
				label: 'Unrecognised',
				headline: 'This import is in a state this page does not recognise.',
				detail: `The server calls it "${stage.state}". Nothing here is lost; this page is behind.`,
				tone: 'mut'
			};
	}
}

/** Whether everything the request named ended up imported. Read by the tone
 *  alone, and false for anything left over, whichever of the three ways a
 *  resource can fail to be imported it took. */
function settledCleanly(counted: ResourceTally): boolean {
	return counted.skipped === 0 && counted.unsettled === 0 && counted.unrecognised === 0;
}

/** The headline for an import that named resources and landed none of them.
 *
 * Every count it carries is one the head already holds. It does not say why any
 * one was skipped, because the head does not know: a reason is recorded per
 * resource and read on the request's own page, so the row says where to look
 * rather than implying it has already looked. */
function nothingImportedHeadline(counted: ResourceTally): string {
	const parts: string[] = [];
	if (counted.skipped > 0) {
		parts.push(`${listings(counted.skipped)} skipped`);
	}
	if (counted.unsettled > 0) {
		parts.push(`${listings(counted.unsettled)} still unsettled`);
	}
	if (counted.unrecognised > 0) {
		parts.push(`${listings(counted.unrecognised)} in a state this page does not recognise`);
	}
	return parts.length === 0
		? 'Nothing was imported.'
		: `Nothing was imported: ${parts.join(', ')}.`;
}

function skippedSentence(counted: ResourceTally): string {
	const parts: string[] = [];
	if (counted.skipped > 0) {
		parts.push(`${listings(counted.skipped)} skipped, each with its reason below.`);
	}
	if (counted.unsettled > 0) {
		parts.push(`${listings(counted.unsettled)} still unsettled.`);
	}
	if (counted.unrecognised > 0) {
		parts.push(`${listings(counted.unrecognised)} in a state this page does not recognise.`);
	}
	return parts.join(' ');
}

/** What the page says when no machine this seller has registered is new enough
 *  to run the work.
 *
 * Null where the field is absent, which is both of its spellings: the work is
 * runnable, and there is nothing to say. Also null for an empty version, which
 * is not a version and would render as an instruction to update to nothing. */
export function deviceUpdateNotice(view: SyncRequestView): string | null {
	// The server derives this field from the request being a device-branch
	// migrate and from the organisation's currently registered devices, never
	// from the request's state, so it survives the import it was about. Its
	// sentence says no machine can run this import, which is false the moment
	// the import has run: a seller who imports on a laptop and later retires
	// that laptop would be told their finished import cannot start.
	if (stageOf(view).kind !== 'waiting_for_device') {
		return null;
	}
	const version = view.waiting_for_device_version;
	if (version === undefined || version === null || version.trim() === '') {
		return null;
	}
	return (
		`Update the Teachouse app on your device to ${version.trim()} or later. ` +
		'None of your machines can run this import until one of them is at that version.'
	);
}

/** The coverage figures a request or a resource carries, or null where none
 *  were taken.
 *
 * Both `null` and an absent field mean nobody measured. An object of zeros
 * means somebody measured and found nothing, and it is returned as itself, so
 * a caller that renders whatever this returns renders the zeros. Substituting
 * zeros for the absent case would state a measurement that was never made.
 *
 * Generic over what is carried, because a request's coverage counts the
 * resources it summed and a resource's does not: the resource is the row, so a
 * row count on it would count itself. */
export function coverageOf<Measured>(carrier: {
	coverage?: Measured | null;
}): Measured | null {
	return carrier.coverage ?? null;
}

export interface CoverageRow {
	label: string;
	value: number;
}

/** The term figures written out, in the order they are arrived at.
 *
 * The labels name what `tam_import::MeasureTotals` counts: terms read off the
 * source listing, those that mapped inbound to a canonical term, the source's
 * own terms that mapped to nothing, and — of those that did map inbound — the
 * ones with no counterpart on the target, which is the loss a seller feels. */
export function termCoverageRows(coverage: SyncTermCoverage): CoverageRow[] {
	return [
		{ label: 'Terms seen', value: coverage.terms_seen },
		{ label: 'Recognised', value: coverage.terms_mapped },
		{ label: 'Not recognised', value: coverage.terms_unmapped },
		{ label: 'No counterpart on the target', value: coverage.terms_uncovered }
	];
}

/** The four term labels, once, for a list that would otherwise repeat them on
 *  every row. Paired with `termCoverageStrip`, which emits the values in the
 *  same order. */
export const TERM_COVERAGE_LEGEND = termCoverageRows({
	terms_seen: 0,
	terms_mapped: 0,
	terms_unmapped: 0,
	terms_uncovered: 0
})
	.map((row) => row.label)
	.join(' · ');

/** One resource's figures as a compact strip, in the legend's order.
 *
 * Joined rather than emitted per figure with a trailing separator: a list of
 * four values ending in a dangling separator reads as output that was cut off,
 * which on a page whose job is to be believed is worse than it sounds. */
export function termCoverageStrip(coverage: SyncTermCoverage): string {
	return termCoverageRows(coverage)
		.map((row) => String(row.value))
		.join(' · ');
}

/** The request's figures: how many listings the sum is over, then the terms. */
export function coverageRows(coverage: SyncCoverageView): CoverageRow[] {
	return [
		{ label: 'Listings measured', value: coverage.rows },
		...termCoverageRows(coverage)
	];
}

export interface ResourceRow {
	ordinal: number;
	locator: string;
	state: SyncResourceState;
	label: string;
	tone: StageTone;
	/** The device's or the server's own words, passed through rather than
	 *  re-worded. Empty where none was recorded. */
	reason: string;
	/** The term counters alone, or null where this resource was not measured,
	 *  which is every skipped one. */
	coverage: SyncTermCoverage | null;
}

/** How each known resource state is worded and toned.
 *
 * Keeping this in step with the database's set is the same manual discipline
 * as `TALLY_BUCKET`, and it lapses the same way, so both reads below fall back
 * rather than indexing into nothing: an unknown state rendered as the literal
 * word `undefined` is a pill that says the console is broken while telling the
 * seller nothing about their listing. */
const RESOURCE_LABEL: Record<SyncResourceState, string> = {
	pending: 'Waiting',
	canonicalised: 'Imported',
	failed: 'Skipped'
};

const RESOURCE_TONE: Record<SyncResourceState, StageTone> = {
	pending: 'mut',
	canonicalised: 'ok',
	failed: 'bad'
};

const UNRECOGNISED_LABEL = 'Unrecognised';

/** One row per resource, in the order the request holds them, which is the
 *  ordinal the server already sorts on. */
export function resourceRows(view: SyncRequestView): ResourceRow[] {
	return view.resources.map((resource) => ({
		ordinal: resource.ordinal,
		locator: resource.locator,
		state: resource.state,
		label: RESOURCE_LABEL[resource.state] ?? UNRECOGNISED_LABEL,
		tone: RESOURCE_TONE[resource.state] ?? 'mut',
		reason: resource.failure_detail ?? '',
		coverage: coverageOf(resource)
	}));
}

// ------------------------------------------------------- starting a migrate

/** Where a migrate can read from, and it is a fact rather than a preference:
 *  `tam_storage::uncaptured_source` admits the three Tes inventories as a
 *  sync's source and refuses TPT and Etsy, so a migrate offered from either
 *  would be refused at submit. Mirrored here because no view serves it.
 *
 *  A `readonly InventoryId[]` requires no exhaustiveness, so this array is not
 *  what stops the lane when an inventory is added in Rust. What stops it is
 *  `MARKETPLACE_OF`, the total `Record<InventoryId, Marketplace>` that
 *  `migrateSource` and `sourcesOn` both read through: a new inventory leaves
 *  that record incomplete and fails the type check there. */
export const MIGRATE_SOURCES: readonly InventoryId[] = ['TesGb', 'TesUs', 'TesNz'];

/** Where a migrate writes to. One value, because TPT is the only marketplace
 *  this console can create a listing on. */
export const MIGRATE_TARGET: InventoryId = 'Tpt';

/** The marketplace the seller can migrate from, or null where they have no
 *  connection on one.
 *
 * Presence rather than health: a connection that needs a fresh sign-in is
 * still the seller's shop, and it is their device that discovers the sign-in
 * is needed, not this page. Both halves must hold — the marketplace has to run
 * on the seller's own device, and a migrate has to be able to read from it. */
export function migrateSource(connections: readonly ConnectionView[]): Marketplace | null {
	const readable = new Set(MIGRATE_SOURCES.map((inventory) => MARKETPLACE_OF[inventory]));
	for (const connection of connections) {
		const marketplace = connection.marketplace;
		if (TRANSPORT_OF[marketplace] === 'SellerDevice' && readable.has(marketplace)) {
			return marketplace;
		}
	}
	return null;
}

/** The sources a migrate offers on one marketplace, in the console's own
 *  platform order, which is what the seller chooses a site from. */
export function sourcesOn(marketplace: Marketplace): InventoryId[] {
	return MIGRATE_SOURCES.filter((inventory) => MARKETPLACE_OF[inventory] === marketplace);
}

/** The submit a migrate makes: no resources, because the seller's device
 *  enumerates the shop and posts what it finds, and a draft, because
 *  publishing is a later decision the seller takes over the imported items
 *  rather than one taken blind before the import has run. */
export function migrateBody(source: InventoryId): SyncRequestBody {
	return {
		source,
		target: MIGRATE_TARGET,
		disposition: 'migrate',
		intent: 'draft',
		resources: []
	};
}

// ---------------------------------------------------- the authorship gate

/** What this console holds about the seller's authorship declaration for the
 *  marketplace a migrate writes to.
 *
 * Three outcomes rather than a boolean, because the two that refuse are
 * different facts and only one of them is about the seller. `undeclared` is
 * the seller not having declared, which `/v1/connections` states outright.
 * `unrecorded` is this console holding no connection for the target at all, or
 * one from a surface that serves no declarations, and it is not a claim about
 * the seller. Both refuse, and the sentence they share says only what is true
 * of both: nothing is on record here. */
export type AuthorshipStanding =
	| { kind: 'declared'; name: string }
	| { kind: 'undeclared' }
	| { kind: 'unrecorded' };

export function targetAuthorship(
	connections: readonly ConnectionView[]
): AuthorshipStanding {
	const marketplace = MARKETPLACE_OF[MIGRATE_TARGET];
	// `?? undefined` so a null reaches the same arm as an absent field. The
	// endpoint omits the key rather than nulling it, so this is unreachable
	// today; it is written because every other optional in this file treats the
	// two spellings alike, and the one place that did not would have thrown
	// inside a $derived and blanked the page rather than degrading.
	const declaration =
		connections.find((entry) => entry.marketplace === marketplace)?.authorship ?? undefined;
	if (declaration === undefined) {
		return { kind: 'unrecorded' };
	}
	return declaration.state === 'declared'
		? { kind: 'declared', name: declaration.name }
		: { kind: 'undeclared' };
}

/** Whether a migrate may be created at all.
 *
 * This is the console's own gate and, until the server's refusal lands with
 * S4, the only one. It earns its place on its own: an undeclared TPT
 * connection reaches the submit, the adapter refuses each listing, and every
 * item settles failed and terminal, so a seller migrating two hundred listings
 * without a declaration burns all two hundred and declaring afterwards brings
 * none of them back. */
export function mayMigrate(standing: AuthorshipStanding): boolean {
	return standing.kind === 'declared';
}

/** The one sentence the gate shows. True of both refusing standings, because
 *  it says nothing is on record rather than that the seller failed to
 *  declare. */
export const AUTHORSHIP_FIRST =
	'Declare who holds the copyright in what you send to TPT before bringing a shop across. ' +
	'With no declaration on record, every listing this import creates is refused and settles ' +
	'failed for good, and declaring afterwards does not bring them back.';

/** Where the declaration is made: the marketplaces screen, which carries the
 *  per-marketplace declaration form. */
export const AUTHORSHIP_HREF = '/marketplaces';

// ------------------------------------------- what the import screen must say

/** The sentence the import screen exists to carry, required by section 4 of
 *  `docs/notes/design/migration-file-routing.md`.
 *
 * Precise about which bytes, because the honest claim is narrower than "we
 * keep nothing": the thumbnail — the file the wire calls the `cover` role —
 * travels inside the page and is stored, and only the listing's own file never
 * reaches us. A broader promise would be a nicer sentence and a false one. */
export const FILES_STAY_ON_YOUR_COMPUTER =
	'Your listing files are read on your own computer and never reach our servers. ' +
	'Only what describes each listing — its details and its thumbnail — is sent here.';

/** Whether this request is the kind this screen was written for.
 *
 * Every sentence on the import screen speaks about the seller's own machine
 * reading their shop, so both halves have to hold: the source must run on that
 * machine, and the request must be the migrate that reads the whole shop.
 * Reading the disposition alone would call a migrate from a server-branch
 * source an import and tell that seller their device did the work.
 *
 * This is the server's own `measured` predicate restated — `jobs.rs`'s
 * `sync_request_view` decides whether coverage exists by the same pair — so
 * the screen claims a device pass under exactly the conditions the server
 * measured one. */
export function isDeviceImport(view: SyncRequestView): boolean {
	return onSellerDevice(view.source) && view.disposition === 'migrate';
}

/** True of both ways a request can fail that predicate — a sync, and a migrate
 *  whose source we work ourselves — so it never tells a seller which one it
 *  was when it does not know. */
export const NOT_AN_IMPORT =
	'This page follows a shop brought across from a marketplace your own computer reads. ' +
	'This request is not one of those, so its progress is on its run rather than here.';

/** What the listings list says while it holds nothing, which depends on why it
 *  holds nothing.
 *
 * "Nothing has arrived yet" is true only while something may still arrive. On
 * a settled request it contradicts the stage directly above it, which is the
 * kind of disagreement that makes a seller distrust both statements. */
export function emptyListingsLine(stage: SyncStage): string {
	switch (stage.kind) {
		case 'waiting_for_device':
		case 'queued':
		case 'importing':
			return 'Nothing has arrived yet.';
		case 'nothing_to_import':
			return 'Your device reported no listings.';
		case 'failed':
			return 'No listings were recorded before the import failed.';
		case 'imported':
		case 'unrecognised':
			return 'No listings are recorded against this import.';
	}
}

// ------------------------------------------------------- naming the sites

/** One site, named by the only part that tells it from its siblings.
 *
 * The three Tes sites differ by region alone, so a group labelled by full
 * platform title reads as the same words three times under a question that has
 * already said which platform it means. */
export function siteLabel(inventory: InventoryId): string {
	return PLATFORMS[inventory].region ?? SHORT_NAME[inventory];
}

/** The question above the site choice, naming the platform once. */
export function siteChoiceQuestion(sites: readonly InventoryId[]): string {
	const first = sites[0];
	const platform = first === undefined ? 'this marketplace' : PLATFORMS[first].acronym;
	return `Which ${platform} site do you sell on?`;
}
