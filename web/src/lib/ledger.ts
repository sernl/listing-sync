// The snapshot-plus-delta store: one EventSource per tab projects the
// ledger, the cursor is the org_seq scalar carried as the SSE event id, and
// a resync event means the replay window was pruned past — refetch
// snapshots rather than trust a partial replay. Framework-free (the
// subscribe contract Svelte auto-subscribes to) so the merge logic tests
// without a component in sight.

import { JOB_EVENT_KINDS } from '$lib/generated/vocab';

export interface LedgerEvent {
	seq: number;
	kind: string;
	payload: unknown;
}

export interface LedgerState {
	cursor: number;
	connected: boolean;
	/** Bumped on resync; consumers refetch their snapshots when it changes. */
	resyncs: number;
	events: LedgerEvent[];
}

type Subscriber = (state: LedgerState) => void;

/** The subset of EventSource the store consumes; tests script a fake. */
export interface EventStream {
	addEventListener(kind: string, handler: (event: MessageEvent) => void): void;
	close(): void;
}

export interface Ledger {
	subscribe(run: Subscriber): () => void;
	/** Feeds one wire event; exported for the fake-driven tests. */
	ingest(kind: string, id: string, data: string): void;
	close(): void;
}

const KEPT_EVENTS = 250;

export function createLedger(
	openStream: (cursor: number) => EventStream,
	initialCursor = 0
): Ledger {
	let state: LedgerState = {
		cursor: initialCursor,
		connected: false,
		resyncs: 0,
		events: []
	};
	const subscribers = new Set<Subscriber>();
	const notify = () => {
		for (const run of subscribers) {
			run(state);
		}
	};

	const ingest = (kind: string, id: string, data: string) => {
		const seq = Number.parseInt(id, 10);
		if (!Number.isFinite(seq)) {
			return;
		}
		if (kind === 'resync') {
			// The pruning watermark passed our cursor: adopt the snapshot
			// cursor and tell consumers to refetch rather than merge.
			state = { ...state, cursor: seq, resyncs: state.resyncs + 1, events: [] };
			notify();
			return;
		}
		if (seq <= state.cursor) {
			return;
		}
		let payload: unknown = null;
		try {
			payload = JSON.parse(data);
		} catch {
			payload = null;
		}
		const events = [...state.events, { seq, kind, payload }].slice(-KEPT_EVENTS);
		state = { ...state, cursor: seq, events };
		notify();
	};

	const stream = openStream(initialCursor);
	for (const kind of ['resync', ...JOB_EVENT_KINDS]) {
		stream.addEventListener(kind, (event) => ingest(kind, event.lastEventId, event.data));
	}
	stream.addEventListener('open', () => {
		state = { ...state, connected: true };
		notify();
	});
	stream.addEventListener('error', () => {
		state = { ...state, connected: false };
		notify();
	});

	return {
		subscribe(run: Subscriber) {
			run(state);
			subscribers.add(run);
			return () => subscribers.delete(run);
		},
		ingest,
		close() {
			stream.close();
		}
	};
}
