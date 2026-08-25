import { describe, expect, it } from 'vitest';
import { visibleWindow } from './window';

describe('the windowed table', () => {
	it('slices the visible rows with overscan and honest padding', () => {
		const win = visibleWindow(1000, 40, 4000, 400, 5);
		expect(win.start).toBe(95);
		expect(win.end).toBe(115);
		expect(win.padTop).toBe(95 * 40);
		expect(win.padBottom).toBe((1000 - 115) * 40);
	});

	it('clamps at both ends', () => {
		const top = visibleWindow(100, 40, 0, 400);
		expect(top.start).toBe(0);
		const bottom = visibleWindow(10, 40, 100000, 400);
		expect(bottom.end).toBe(10);
	});

	it('an empty table windows to nothing', () => {
		expect(visibleWindow(0, 40, 0, 400)).toEqual({
			start: 0,
			end: 0,
			padTop: 0,
			padBottom: 0
		});
	});
});
