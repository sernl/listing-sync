// The Automations landing: the three automations this console offers, what
// each one does for a teacher who sells resources, and the condition it is
// honestly in right now. Pure, so it tests without a component.
//
// The state is computed rather than written down, because a card that always
// reads "Ready" is a card that has stopped meaning anything. Each answer comes
// from a read the page already makes for another reason.

import type { ConnectionView, SyncRequestHead } from '$lib/api';
import { anyConnectionStands } from '$lib/connection-standing';
import type { IconName } from '$lib/icons';
import { headStage, mayMigrate, migrateSource, targetAuthorship } from '$lib/sync-request';

/** The tones a landing badge takes. A subset of the status pill's own tones:
 *  `bad` belongs to a connection that is gone and `flat` to a transport class,
 *  and an automation is neither. Assignability to the pill's prop is what
 *  keeps the two in step, so this narrowing costs nothing. */
export type CardTone = 'ok' | 'warn' | 'run' | 'soon';

export interface CardState {
	label: string;
	tone: CardTone;
}

export type AutomationId = 'sharing' | 'migration' | 'sync';

export interface AutomationCard {
	id: AutomationId;
	href: string;
	title: string;
	icon: IconName;
	/** What this automation does for the seller, in one paragraph and in the
	 *  words a teacher would use for it. */
	what: string;
	state: CardState;
}

/** Everything the landing needs to state each automation's condition, taken
 *  from the three reads the page makes. */
export interface AutomationFacts {
	connections: readonly ConnectionView[];
	requests: readonly SyncRequestHead[];
	/** Reconciliation questions still open, or null where the figure could not
	 *  be read. A count this console failed to read is not a count of zero. */
	openQuestions: number | null;
}

const SHARING_WHAT =
	'Publish one resource to every marketplace you are connected to, in one scheduled action.';

const MIGRATION_WHAT =
	'Move a whole shop from one marketplace to another, once. Everything arrives as a draft ' +
	'for you to check before anything is published.';

const SYNC_WHAT =
	'Keep every marketplace’s copy of a resource up to date. Change a price or a description ' +
	'here and it is carried out to each marketplace that has it.';

/** Whether a request is still doing something, read off the stage the request
 *  list already presents rather than off the raw state, so this page and the
 *  request's own page can never disagree about what "running" means. */
function running(head: SyncRequestHead): boolean {
	const stage = headStage(head);
	return (
		stage.kind === 'waiting_for_device' ||
		stage.kind === 'queued' ||
		stage.kind === 'importing'
	);
}

function plural(count: number, one: string, many: string): string {
	return count === 1 ? `1 ${one}` : `${count} ${many}`;
}

/** Sharing has no foundation at all: no share, bump or relist concept exists
 *  anywhere behind this console, so its state is a constant rather than a
 *  reading. */
export function sharingState(): CardState {
	return { label: 'Coming soon', tone: 'soon' };
}

export function migrationState(facts: AutomationFacts): CardState {
	const inFlight = migrations(facts.requests).filter(running).length;
	if (inFlight > 0) {
		return { label: plural(inFlight, 'running', 'running'), tone: 'run' };
	}
	if (migrateSource(facts.connections) === null) {
		return { label: 'No shop connected', tone: 'warn' };
	}
	// TPT named here rather than left to the seller's choice, because this card
	// states one summary for a page where the pair is not yet chosen and TPT is
	// the only target a migration can be authored to today. A card that said
	// "Ready" while the one available target refused every listing for a
	// missing declaration would be the wrong summary of the two.
	if (!mayMigrate(targetAuthorship(facts.connections, 'Tpt'))) {
		return { label: 'Copyright needed', tone: 'warn' };
	}
	return { label: 'Ready', tone: 'ok' };
}

export function syncState(facts: AutomationFacts): CardState {
	// The row's presence is not the question: a connection the seller
	// disconnected is still a row, and reading it as a marketplace they have is
	// exactly what `$lib/connection-standing` exists to stop. The migration and
	// import pages ask it this way too.
	if (!anyConnectionStands(facts.connections)) {
		return { label: 'No marketplace connected', tone: 'warn' };
	}
	const open = facts.openQuestions;
	// A third state rather than falling through to "Ready": the doc comment on
	// `openQuestions` says a figure this console failed to read is not a figure
	// of zero, and collapsing the two here would make that distinction one the
	// result cannot express.
	if (open === null) {
		return { label: 'Questions unknown', tone: 'soon' };
	}
	if (open > 0) {
		return { label: plural(open, 'open question', 'open questions'), tone: 'warn' };
	}
	return { label: 'Ready', tone: 'ok' };
}

/** Only the migrate half of the request list. A sync request and a migration
 *  are the same record under two dispositions, and each Automations page owns
 *  one of them. */
export function migrations(requests: readonly SyncRequestHead[]): SyncRequestHead[] {
	return requests.filter((row) => row.disposition === 'migrate');
}

export function cards(facts: AutomationFacts): AutomationCard[] {
	return [
		{
			id: 'sharing',
			href: '/automations/sharing',
			title: 'Marketplace Sharing',
			icon: 'share-2',
			what: SHARING_WHAT,
			state: sharingState()
		},
		{
			id: 'migration',
			href: '/automations/migration',
			title: 'Migrations',
			icon: 'arrow-right-left',
			what: MIGRATION_WHAT,
			state: migrationState(facts)
		},
		{
			id: 'sync',
			href: '/sync',
			title: 'Marketplace Sync',
			icon: 'refresh-cw',
			what: SYNC_WHAT,
			state: syncState(facts)
		}
	];
}
