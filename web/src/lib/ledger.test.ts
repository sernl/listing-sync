import { describe, expect, it } from 'vitest';
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

	it('closes its stream when closed', () => {
		const { stream, ledger } = harness();
		ledger.close();
		expect(stream.closed).toBe(true);
	});
});
