import { describe, expect, it } from 'vitest';
import { UNREAD, sumOf, tileFor, type Tile } from './ledger-tile';

const RAISED: Tile = { icon: 'circle-x', tone: 'bad' };
const CLEAR: Tile = { icon: 'circle-check', tone: 'ok' };

describe('a counted tile’s glyph and tint', () => {
	it('raises on a count above zero', () => {
		expect(tileFor(1, RAISED, CLEAR)).toEqual(RAISED);
		expect(tileFor(33, RAISED, CLEAR)).toEqual(RAISED);
	});

	it('clears on a count of exactly zero', () => {
		expect(tileFor(0, RAISED, CLEAR)).toEqual(CLEAR);
	});

	// The defect this function exists to close: written as a two-way choice,
	// an unread figure falls into the clear arm, so the card paints a green
	// tick above the em dash its own value slot prints.
	it('takes neither arm where the figure was never read', () => {
		expect(tileFor(undefined, RAISED, CLEAR)).toEqual(UNREAD);
	});

	it('never reports an unread figure as healthy', () => {
		expect(tileFor(undefined, RAISED, CLEAR).tone).not.toBe('ok');
		expect(tileFor(undefined, RAISED, CLEAR).icon).not.toBe('circle-check');
	});

	// The settled tile's clear arm is itself `ok`, because zero settled items
	// is not a fault. The unread answer has to stay neutral even then.
	it('stays neutral even where both arms are benign', () => {
		expect(tileFor(undefined, CLEAR, { icon: 'minus', tone: 'ok' })).toEqual(UNREAD);
	});
});

describe('a total built from several figures', () => {
	it('adds them where every one is present', () => {
		expect(sumOf(12, 3, 5, 2)).toBe(22);
		expect(sumOf()).toBe(0);
	});

	// The defect: a sum containing undefined is NaN, which `?? '—'` does not
	// catch and which `tileFor` sends to its clear arm — a green tick over the
	// literal NaN.
	it('is absent where any one figure is', () => {
		expect(sumOf(12, undefined, 5)).toBeUndefined();
		expect(sumOf(undefined)).toBeUndefined();
	});

	it('is absent rather than NaN, so the tile reads it as unread', () => {
		const total = sumOf(4, undefined);
		expect(Number.isNaN(total as number)).toBe(false);
		expect(tileFor(total, RAISED, CLEAR)).toEqual(UNREAD);
	});
});
