import { JOB_EVENT_KINDS } from '$lib/generated/vocab';

export interface LedgerState {
	cursor: number;
	connected: boolean;
	/** Changes only when a coalesced event batch needs a new snapshot. */
	revision: number;
	resyncs: number;
	kinds: ReadonlySet<string>;
}

type Subscriber = (state: LedgerState) => void;

export interface EventStream {
	addEventListener(kind: string, handler: (event: MessageEvent) => void): void;
	close(): void;
}

export interface Ledger {
	subscribe(run: Subscriber): () => void;
	close(): void;
}

let scope: string | null = null;
let rememberedCursor = 0;
const active = new Set<Ledger>();

/** A cursor is valid only for the authenticated organisation that issued it. */
export function setLedgerScope(org: string | null): void {
	if (org !== null && org === scope) return;
	for (const ledger of active) ledger.close();
	scope = org;
	rememberedCursor = 0;
}

/** Open before requesting the snapshot, so no event can fall between the two. */
export function createLedger(
	openStream: (cursor: number) => EventStream,
	initialCursor = rememberedCursor
): Ledger {
	let state: LedgerState = {
		cursor: initialCursor,
		connected: false,
		revision: 0,
		resyncs: 0,
		kinds: new Set()
	};
	const subscribers = new Set<Subscriber>();
	let pendingKinds = new Set<string>();
	let pendingResyncs = 0;
	let timer: number | NodeJS.Timeout | undefined;
	let closed = false;
	const notify = () => {
		for (const run of subscribers) run(state);
	};
	const stream = openStream(initialCursor);
	for (const kind of ['resync', ...JOB_EVENT_KINDS]) {
		stream.addEventListener(kind, (event) => {
			if (closed || !/^\d+$/.test(event.lastEventId)) return;
			const seq = Number(event.lastEventId);
			if (!Number.isSafeInteger(seq) || seq < 0) return;
			if (kind !== 'resync' && seq <= state.cursor) return;
			state = { ...state, cursor: Math.max(state.cursor, seq) };
			rememberedCursor = Math.max(rememberedCursor, state.cursor);
			pendingKinds.add(kind);
			if (kind === 'resync') pendingResyncs += 1;
			if (timer !== undefined) return;
			// A fixed window coalesces replay without starving progress on a busy stream.
			timer = setTimeout(() => {
				timer = undefined;
				state = {
					...state,
					revision: state.revision + 1,
					resyncs: state.resyncs + pendingResyncs,
					kinds: pendingKinds
				};
				pendingKinds = new Set();
				pendingResyncs = 0;
				notify();
			}, 100);
		});
	}
	stream.addEventListener('open', () => {
		if (closed) return;
		state = { ...state, connected: true };
		notify();
	});
	stream.addEventListener('error', () => {
		if (closed) return;
		state = { ...state, connected: false };
		notify();
	});
	const ledger: Ledger = {
		subscribe(run) {
			run(state);
			subscribers.add(run);
			return () => subscribers.delete(run);
		},
		close() {
			closed = true;
			clearTimeout(timer);
			subscribers.clear();
			stream.close();
			active.delete(ledger);
		}
	};
	active.add(ledger);
	return ledger;
}
