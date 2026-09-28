import { describe, expect, it } from 'vitest';
import {
	NOTHING,
	afterSearch,
	isSelected,
	pageTick,
	selectMatching,
	selectedCount,
	targets,
	toggle,
	togglePage
} from './user-selection';

const page1 = ['a', 'b', 'c'];
const page2 = ['d', 'e'];
const NOW = Date.UTC(2026, 8, 1);
const rows = (...ids: string[]) => ids.map((id) => ({ id }));

describe('ticking rows one at a time', () => {
	it('ticks and unticks the same row', () => {
		const once = toggle(NOTHING, 'a');
		expect(isSelected(once, 'a')).toBe(true);
		expect(selectedCount(once)).toBe(1);
		const twice = toggle(once, 'a');
		expect(isSelected(twice, 'a')).toBe(false);
		expect(selectedCount(twice)).toBe(0);
	});

	it('does not change the selection it was given', () => {
		const before = toggle(NOTHING, 'a');
		toggle(before, 'b');
		expect(selectedCount(before)).toBe(1);
	});
});

describe('the header checkbox', () => {
	it('reads none, some and all for the rows on screen', () => {
		expect(pageTick(NOTHING, page1)).toBe('none');
		expect(pageTick(toggle(NOTHING, 'b'), page1)).toBe('some');
		expect(pageTick(togglePage(NOTHING, page1), page1)).toBe('all');
	});

	it('reads none on an empty page rather than all', () => {
		expect(pageTick(NOTHING, [])).toBe('none');
	});

	it('ticks the rest of a partly ticked page, then unticks the whole page', () => {
		const partly = toggle(NOTHING, 'b');
		const all = togglePage(partly, page1);
		expect(page1.every((id) => isSelected(all, id))).toBe(true);
		const none = togglePage(all, page1);
		expect(selectedCount(none)).toBe(0);
	});

	it('keeps rows ticked on another page', () => {
		const first = togglePage(NOTHING, page1);
		const both = togglePage(first, page2);
		expect(selectedCount(both)).toBe(5);
		const firstOnly = togglePage(both, page2);
		expect(selectedCount(firstOnly)).toBe(3);
		expect(isSelected(firstOnly, 'a')).toBe(true);
	});
});

describe('everyone the search matches', () => {
	it('counts the whole match, not the page', () => {
		const all = selectMatching('maths', 240, NOW);
		expect(selectedCount(all)).toBe(240);
		expect(isSelected(all, 'someone-on-page-9')).toBe(true);
	});

	it('records unticked rows as exceptions', () => {
		const all = toggle(selectMatching('maths', 240, NOW), 'a');
		expect(isSelected(all, 'a')).toBe(false);
		expect(selectedCount(all)).toBe(239);
		expect(pageTick(all, page1)).toBe('some');
		expect(isSelected(toggle(all, 'a'), 'a')).toBe(true);
	});

	it('unticking the page adds its rows as exceptions, and ticking it takes them back', () => {
		const off = togglePage(selectMatching('', 10, NOW), page1);
		expect(selectedCount(off)).toBe(7);
		expect(pageTick(off, page1)).toBe('none');
		expect(selectedCount(togglePage(off, page1))).toBe(10);
	});

	it('becomes nothing once every match is unticked', () => {
		const one = selectMatching('', 1, NOW);
		expect(toggle(one, 'a')).toEqual(NOTHING);
		expect(togglePage(selectMatching('', 3, NOW), page1)).toEqual(NOTHING);
	});

	it('selects nothing when the search matches nobody', () => {
		expect(selectMatching('nobody', 0, NOW)).toEqual(NOTHING);
	});

	it('does not carry over to a different search, but ticked ids do', () => {
		const all = selectMatching('maths', 240, NOW);
		expect(afterSearch(all, 'maths')).toBe(all);
		expect(afterSearch(all, 'art')).toEqual(NOTHING);
		const ticked = toggle(NOTHING, 'a');
		expect(afterSearch(ticked, 'art')).toBe(ticked);
	});
});

describe('what a bulk action runs on', () => {
	it('is the ticked ids, among the accounts read', () => {
		const selection = toggle(toggle(NOTHING, 'b'), 'd');
		expect(targets(selection, rows('a', 'b', 'c', 'd'))).toEqual(rows('b', 'd'));
	});

	it('is every match but the exceptions', () => {
		const selection = toggle(selectMatching('', 4, NOW), 'c');
		expect(targets(selection, rows('a', 'b', 'c', 'd'))).toEqual(rows('a', 'b', 'd'));
	});

	it('leaves out accounts made after the operator chose them', () => {
		// Someone signed up between "select all 2 matching" and the confirm.
		const selection = selectMatching('', 2, NOW);
		const read = [
			{ id: 'new', createdAt: new Date(NOW + 1).toISOString() },
			{ id: 'a', createdAt: new Date(NOW - 1).toISOString() },
			{ id: 'b', createdAt: new Date(NOW).toISOString() }
		];
		expect(targets(selection, read).map((row) => row.id)).toEqual(['a', 'b']);
	});
});
