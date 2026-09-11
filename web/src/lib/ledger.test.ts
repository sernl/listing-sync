import { describe, expect, it } from 'vitest';
import { JOB_EVENT_KINDS } from './generated/vocab';
import { createLedger, type EventStream, type LedgerState } from './ledger';

class FakeStream implements EventStream {
	handlers = new Map<string, (event: MessageEvent) => void>();
	closed = false;

	addEventListener(kind: string, handler: (event: MessageEvent) => void) {
		this.handlers.set(kind, handler);
	}

	close() {
		this.closed = true;
	}

	fire(kind: string, id: string, data: string) {
		this.handlers.get(kind)?.({ lastEventId: id, data } as MessageEvent);
	}
}

function harness(initial = 0) {
	const stream = new FakeStream();
	const ledger = createLedger(() => stream, initial);
	let state!: LedgerState;
	ledger.subscribe((next) => {
		state = next;
	});
	return { stream, ledger, state: () => state };
}

describe('the ledger store', () => {
	it('merges deltas in cursor order and drops replays', () => {
		const { stream, state } = harness();
		stream.fire('JobQueued', '1', '{"items":2}');
		stream.fire('ItemSettled', '2', '{"outcome":"succeeded"}');
		stream.fire('JobQueued', '1', '{"items":2}');
		expect(state().cursor).toBe(2);
		expect(state().events.map((event) => event.seq)).toEqual([1, 2]);
	});

	it('treats a resync as adopt-and-refetch, not a merge', () => {
		const { stream, state } = harness(1);
		stream.fire('JobQueued', '2', '{}');
		stream.fire('resync', '9', '{"cursor":9}');
		expect(state().cursor).toBe(9);
		expect(state().resyncs).toBe(1);
		expect(state().events).toEqual([]);
	});

	it('ignores an unparseable id rather than corrupting the cursor', () => {
		const { stream, state } = harness();
		stream.fire('JobQueued', 'not-a-number', '{}');
		expect(state().cursor).toBe(0);
		expect(state().events).toEqual([]);
	});

	// Two assertions doing two jobs, because the first alone is weaker than it
	// looks. Comparing the handler keys against JOB_EVENT_KINDS cannot fail
	// while `createLedger` derives its loop from that same constant: it catches
	// the subscription being rewritten as a hand-kept list, and nothing else.
	it('registers a listener for exactly resync, the connection signals and every generated kind', () => {
		const { stream } = harness();
		expect([...stream.handlers.keys()].sort()).toEqual(
			['resync', 'open', 'error', ...JOB_EVENT_KINDS].sort()
		);
	});

	// So the kinds the import screen's liveness actually depends on are named
	// as literals, which the implementation cannot satisfy by construction. If
	// a regeneration drops them from the vocabulary, the console stops hearing
	// the events that move that page and nothing else would say so.
	it('listens for the two import events the import screen depends on', () => {
		const { stream } = harness();
		expect(stream.handlers.has('ImportPageApplied')).toBe(true);
		expect(stream.handlers.has('ImportCompleted')).toBe(true);
	});

	it('parses a generated event onto the state under its own kind', () => {
		const { stream, state } = harness();
		stream.fire(
			'ImportDrainMeasured',
			'1',
			'{"source":"Tes","target":"Tpt","rows":1,"terms_seen":3,' +
				'"terms_unmapped":1,"terms_covered":1,"items_new":1,"items_already_open":0}'
		);
		expect(state().events).toHaveLength(1);
		expect(state().events[0].kind).toBe('ImportDrainMeasured');
		expect((state().events[0].payload as { items_new: number }).items_new).toBe(1);
	});

	it('closes its stream when closed', () => {
		const { stream, ledger } = harness();
		ledger.close();
		expect(stream.closed).toBe(true);
	});
});
