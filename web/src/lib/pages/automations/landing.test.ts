import { describe, expect, it } from 'vitest';
import type { AutomationFacts } from './landing';
import { cards, migrationState, sharingState, syncState } from './landing';
import { DECLARED, connection, request } from './fixtures.test-support';

function facts(over: Partial<AutomationFacts> = {}): AutomationFacts {
	return { connections: [], requests: [], openQuestions: 0, ...over };
}

/** A seller who can start a migration: a Tes shop to read, and authorship
 *  declared for the marketplace the work is published to. */
const READY = facts({
	connections: [connection('Tes'), connection('Tpt', { authorship: DECLARED })]
});

describe('the landing cards', () => {
	it('offers the three automations, in the order the navigation lists them', () => {
		expect(cards(facts()).map((card) => card.id)).toEqual(['sharing', 'migration', 'sync']);
	});

	it('sends Marketplace Sync to the list that already exists rather than to a new path', () => {
		const sync = cards(facts()).find((card) => card.id === 'sync');
		expect(sync?.href).toBe('/sync');
	});

	it('says what each automation does for the seller rather than naming a feature', () => {
		for (const card of cards(facts())) {
			expect(card.what.length).toBeGreaterThan(120);
		}
	});
});

describe('the sharing card', () => {
	it('reports the same thing whatever the seller has connected, because none of it is built', () => {
		expect(sharingState()).toEqual({ label: 'Coming soon', tone: 'soon' });
	});
});

describe('the migration card', () => {
	it('reads as ready only when a shop can be read and authorship stands', () => {
		expect(migrationState(READY)).toEqual({ label: 'Ready', tone: 'ok' });
	});

	it('says a shop is needed before it says a declaration is', () => {
		expect(migrationState(facts()).label).toBe('No shop connected');
	});

	it('names the declaration once there is a shop to move', () => {
		expect(migrationState(facts({ connections: [connection('Tes')] })).label).toBe(
			'Authorship needed'
		);
	});

	it('reports what is running ahead of what is missing', () => {
		const running = facts({
			connections: [],
			requests: [request({ state: 'draining', resources_total: 2 })]
		});
		expect(migrationState(running)).toEqual({ label: '1 running', tone: 'run' });
	});

	it('counts a migrate still waiting for a device as running', () => {
		const waiting = facts({
			requests: [request({ state: 'pending', resources_total: 0 })]
		});
		expect(migrationState(waiting).tone).toBe('run');
	});

	it('does not count a finished migration as running', () => {
		const done = facts({
			connections: [connection('Tes'), connection('Tpt', { authorship: DECLARED })],
			requests: [request({ state: 'enqueued' })]
		});
		expect(migrationState(done)).toEqual({ label: 'Ready', tone: 'ok' });
	});

	it('ignores a sync request, which belongs to the other page', () => {
		const synced = facts({
			connections: [connection('Tes'), connection('Tpt', { authorship: DECLARED })],
			requests: [request({ disposition: 'sync', state: 'draining' })]
		});
		expect(migrationState(synced).label).toBe('Ready');
	});
});

describe('the sync card', () => {
	it('asks for a marketplace before it reports on questions', () => {
		expect(syncState(facts({ openQuestions: 3 })).label).toBe('No marketplace connected');
	});

	it('raises the open questions where there are any', () => {
		const asked = facts({ connections: [connection('Tpt')], openQuestions: 3 });
		expect(syncState(asked)).toEqual({ label: '3 open questions', tone: 'warn' });
	});

	it('counts one question in the singular', () => {
		const asked = facts({ connections: [connection('Tpt')], openQuestions: 1 });
		expect(syncState(asked).label).toBe('1 open question');
	});

	it('does not read a figure it could not fetch as a figure of zero', () => {
		const unread = facts({ connections: [connection('Tpt')], openQuestions: null });
		const none = facts({ connections: [connection('Tpt')], openQuestions: 0 });
		expect(syncState(unread)).toEqual({ label: 'Questions unknown', tone: 'soon' });
		// The assertion the name actually claims: the two must not agree, which
		// is what `(openQuestions ?? 0)` would silently break.
		expect(syncState(unread)).not.toEqual(syncState(none));
	});

	it('reads a queue that is genuinely drained as ready', () => {
		const none = facts({ connections: [connection('Tpt')], openQuestions: 0 });
		expect(syncState(none)).toEqual({ label: 'Ready', tone: 'ok' });
	});
});
