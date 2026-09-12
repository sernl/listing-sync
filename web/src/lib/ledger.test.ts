import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createLedger, setLedgerScope, type EventStream, type LedgerState } from './ledger';

class FakeStream implements EventStream {
	handlers = new Map<string, (event: MessageEvent) => void>();
	closed = false;

	addEventListener(kind: string, handler: (event: MessageEvent) => void) {
		this.handlers.set(kind, handler);
	}

	close() {
		this.closed = true;
	}

	fire(kind: string, id = '') {
		this.handlers.get(kind)?.({ lastEventId: id, data: '{}' } as MessageEvent);
	}
}

beforeEach(() => {
	vi.useFakeTimers();
	setLedgerScope(null);
});
afterEach(() => {
	setLedgerScope(null);
	vi.useRealTimers();
});

function harness(initial = 0) {
	const stream = new FakeStream();
	const ledger = createLedger(() => stream, initial);
	let state!: LedgerState;
	ledger.subscribe((next) => { state = next; });
	return { stream, ledger, state: () => state };
}

describe('the ledger invalidation stream', () => {
	it('coalesces a replay burst into one snapshot invalidation', () => {
		const { stream, ledger } = harness();
		const invalidated: number[] = [];
		ledger.subscribe((state) => {
			if (state.cursor > 0) invalidated.push(state.cursor);
		});
		for (let seq = 1; seq <= 34; seq += 1) stream.fire('ImportRunProgress', String(seq));
		vi.runAllTimers();
		expect(invalidated).toEqual([34]);
	});

	it('ignores replay and connection noise without losing later progress', () => {
		const { stream, ledger } = harness();
		const invalidated: number[] = [];
		let revision = 0;
		ledger.subscribe((state) => {
			if (revision === state.revision) return;
			revision = state.revision;
			invalidated.push(state.cursor);
		});
		stream.fire('ImportRunProgress', '2');
		stream.fire('ImportRunListed', '1');
		vi.runAllTimers();
		stream.fire('error');
		stream.fire('open');
		stream.fire('ImportRunProgress', '2');
		vi.runAllTimers();
		stream.fire('ImportRunSettled', '3');
		vi.runAllTimers();
		expect(invalidated).toEqual([2, 3]);
	});

	it('requires a fresh snapshot after pruning without moving its cursor backwards', () => {
		const { stream, state } = harness(12);
		stream.fire('resync', '9');
		vi.runAllTimers();
		expect(state().cursor).toBe(12);
		expect(state().resyncs).toBe(1);
		expect(state().kinds.has('resync')).toBe(true);
	});

	it('resumes a remount within one organisation but not across sessions', () => {
		setLedgerScope('first-org');
		const first = harness();
		first.stream.fire('ImportRunListed', '8');
		first.ledger.close();
		const resumed: number[] = [];
		createLedger((cursor) => { resumed.push(cursor); return new FakeStream(); });
		expect(resumed).toEqual([8]);
		setLedgerScope('second-org');
		first.stream.fire('ImportRunProgress', '99');
		createLedger((cursor) => { resumed.push(cursor); return new FakeStream(); });
		expect(resumed).toEqual([8, 0]);
		setLedgerScope(null);
		setLedgerScope('first-org');
		createLedger((cursor) => { resumed.push(cursor); return new FakeStream(); });
		expect(resumed).toEqual([8, 0, 0]);
	});

	it('does not let malformed event identifiers hide a valid later event', () => {
		const { stream, state } = harness();
		for (const id of ['9junk', '-1', '1.2', '9007199254740992']) {
			stream.fire('ImportRunProgress', id);
		}
		stream.fire('ImportRunProgress', '4');
		vi.runAllTimers();
		expect(state().cursor).toBe(4);
		expect(state().revision).toBe(1);
	});
});
