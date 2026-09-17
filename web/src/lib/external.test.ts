import { afterEach, describe, expect, it, vi } from 'vitest';
import { OPEN_URL } from './desktop';
import { external } from './external';

afterEach(() => {
	vi.unstubAllGlobals();
});

/** An anchor, without a DOM. The suite runs in the node environment, and the
 *  three things this action touches on an element — its resolved `href`, its
 *  `target`, and one click listener — are the whole of what a real one would
 *  contribute. */
function anchor(href: string, target: string) {
	const listeners: Array<(event: MouseEvent) => void> = [];
	const node = {
		href,
		target,
		addEventListener(name: string, fn: (event: MouseEvent) => void) {
			expect(name).toBe('click');
			listeners.push(fn);
		},
		removeEventListener(name: string, fn: (event: MouseEvent) => void) {
			const at = listeners.indexOf(fn);
			if (at >= 0) {
				listeners.splice(at, 1);
			}
		}
	};
	return { node: node as unknown as HTMLAnchorElement, listeners };
}

function press() {
	const event = {
		defaultPrevented: false,
		preventDefault() {
			event.defaultPrevented = true;
		}
	};
	return event as unknown as MouseEvent & { defaultPrevented: boolean };
}

/** Let the handler's own promise settle. It deliberately does not return one:
 *  a click handler that awaited would hold the event loop for as long as the
 *  application took to answer. */
const settled = () => new Promise((resume) => setTimeout(resume, 0));

describe('a link out of the console', () => {
	it('is left entirely alone in a browser', () => {
		const { node, listeners } = anchor('https://www.tes.com/', '_blank');
		expect(external(node)).toBeUndefined();
		expect(listeners).toHaveLength(0);
	});

	it('goes to the browser this computer already uses when there is an application to ask', async () => {
		const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
		const open = vi.fn();
		vi.stubGlobal('window', {
			open,
			__TAURI__: {
				core: {
					invoke: async (command: string, args: Record<string, unknown>) => {
						calls.push({ command, args });
						return null;
					}
				}
			}
		});
		const { node, listeners } = anchor('https://www.tes.com/teaching-resource/x-1', '_blank');
		external(node);
		const event = press();
		listeners[0](event);
		await settled();
		expect(event.defaultPrevented).toBe(true);
		expect(calls).toEqual([
			{ command: OPEN_URL, args: { url: 'https://www.tes.com/teaching-resource/x-1' } }
		]);
		// The whole point of taking the click: nothing opens inside the
		// application window.
		expect(open).not.toHaveBeenCalled();
	});

	it('falls back to what the anchor did before, when the application refuses', async () => {
		const open = vi.fn();
		vi.stubGlobal('window', {
			open,
			__TAURI__: {
				core: {
					invoke: async () => {
						throw 'Forbidden URL';
					}
				}
			}
		});
		const { node, listeners } = anchor('https://www.teacherspayteachers.com/', '_blank');
		external(node);
		listeners[0](press());
		await settled();
		expect(open).toHaveBeenCalledWith('https://www.teacherspayteachers.com/', '_blank', 'noopener');
	});

	it('leaves an anchor that stays in the console alone', async () => {
		// The resource detail renders one anchor whose action is external for a
		// listed marketplace and internal otherwise. Its address is served from
		// our own https origin, so the plugin's scope would happily open it: the
		// target is the only thing that tells the two apart.
		const invoke = vi.fn();
		const open = vi.fn();
		vi.stubGlobal('window', { open, __TAURI__: { core: { invoke } } });
		const { node, listeners } = anchor('https://teachouse.io/resources/x-1', '');
		external(node);
		const event = press();
		listeners[0](event);
		await settled();
		expect(event.defaultPrevented).toBe(false);
		expect(invoke).not.toHaveBeenCalled();
		expect(open).not.toHaveBeenCalled();
	});

	it('leaves a click something else has already handled alone', async () => {
		const invoke = vi.fn();
		vi.stubGlobal('window', { open: vi.fn(), __TAURI__: { core: { invoke } } });
		const { node, listeners } = anchor('https://www.tes.com/', '_blank');
		external(node);
		const event = press();
		event.preventDefault();
		listeners[0](event);
		await settled();
		expect(invoke).not.toHaveBeenCalled();
	});

	it('gives the listener back when the element goes', () => {
		vi.stubGlobal('window', { open: vi.fn(), __TAURI__: { core: { invoke: vi.fn() } } });
		const { node, listeners } = anchor('https://www.tes.com/', '_blank');
		external(node)?.destroy();
		expect(listeners).toHaveLength(0);
	});
});

