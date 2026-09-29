import { describe, expect, it } from 'vitest';
import { afterLoadIdle, IDLE_TIMEOUT_MS, INTERACTION_EVENTS, onFirstInteraction, type IdleHost } from './idle';

/** A window with the listeners, idle callbacks and timers held for the test
 *  to fire by hand. */
function stubHost(readyState: DocumentReadyState, idleCallback: boolean) {
	const listeners = new Map<string, Set<() => void>>();
	const idle: { callback: () => void; timeout: number }[] = [];
	const timers: (() => void)[] = [];
	const host: IdleHost = {
		document: { readyState },
		addEventListener: (type, listener) => {
			listeners.set(type, (listeners.get(type) ?? new Set()).add(listener));
		},
		removeEventListener: (type, listener) => {
			listeners.get(type)?.delete(listener);
		},
		setTimeout: (callback) => timers.push(callback),
		...(idleCallback
			? {
					requestIdleCallback: (callback: () => void, options: { timeout: number }) =>
						idle.push({ callback, timeout: options.timeout })
				}
			: {})
	};
	const dispatch = (type: string) => {
		for (const listener of [...(listeners.get(type) ?? [])]) {
			listener();
		}
	};
	const count = () => [...listeners.values()].reduce((sum, set) => sum + set.size, 0);
	return { host, dispatch, idle, timers, count };
}

/** Whether the promise has settled, after letting its queued reactions run:
 *  two microtask turns, one for the resolution and one for the reaction. */
async function settled(promise: Promise<void>): Promise<boolean> {
	let done = false;
	void promise.then(() => (done = true));
	await Promise.resolve();
	await Promise.resolve();
	return done;
}

describe('afterLoadIdle', () => {
	it('waits for load, then for idle, on a page still loading', async () => {
		const stub = stubHost('interactive', true);
		const waiting = afterLoadIdle(stub.host);
		expect(stub.idle).toHaveLength(0);
		expect(await settled(waiting)).toBe(false);

		stub.dispatch('load');
		expect(stub.idle).toHaveLength(1);
		expect(stub.idle[0].timeout).toBe(IDLE_TIMEOUT_MS);
		expect(await settled(waiting)).toBe(false);

		stub.idle[0].callback();
		expect(await settled(waiting)).toBe(true);
	});

	it('asks for idle at once on a page that has already loaded', async () => {
		const stub = stubHost('complete', true);
		const waiting = afterLoadIdle(stub.host);
		expect(stub.idle).toHaveLength(1);
		stub.idle[0].callback();
		expect(await settled(waiting)).toBe(true);
	});

	it('falls back to the next task where the engine has no idle callback', async () => {
		const stub = stubHost('loading', false);
		const waiting = afterLoadIdle(stub.host);
		stub.dispatch('load');
		expect(stub.timers).toHaveLength(1);
		expect(await settled(waiting)).toBe(false);
		stub.timers[0]();
		expect(await settled(waiting)).toBe(true);
	});
});

describe('onFirstInteraction', () => {
	it('runs once, on whichever interaction comes first, and then stops listening', () => {
		for (const first of INTERACTION_EVENTS) {
			const stub = stubHost('complete', true);
			let runs = 0;
			onFirstInteraction(stub.host, () => (runs += 1));
			expect(stub.count()).toBe(INTERACTION_EVENTS.length);

			stub.dispatch(first);
			for (const later of INTERACTION_EVENTS) {
				stub.dispatch(later);
			}
			expect(runs, first).toBe(1);
			expect(stub.count(), first).toBe(0);
		}
	});

	it('never runs once cancelled, and leaves no listener behind', () => {
		const stub = stubHost('complete', true);
		let runs = 0;
		const cancel = onFirstInteraction(stub.host, () => (runs += 1));
		cancel();
		for (const type of INTERACTION_EVENTS) {
			stub.dispatch(type);
		}
		expect(runs).toBe(0);
		expect(stub.count()).toBe(0);
	});

	it('does not count looking as doing', () => {
		const stub = stubHost('complete', true);
		let runs = 0;
		onFirstInteraction(stub.host, () => (runs += 1));
		for (const type of ['scroll', 'pointermove', 'mousemove', 'visibilitychange']) {
			stub.dispatch(type);
		}
		expect(runs).toBe(0);
	});
});
