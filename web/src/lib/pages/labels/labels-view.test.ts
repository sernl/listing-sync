import { describe, expect, it } from 'vitest';
import { countAll } from './api';
import {
	LABEL_MAX_CHARS,
	checkRename,
	countLine,
	deleteWarning,
	filterHref,
	matching,
	rows,
	swatchClass,
	totalLine,
	unchanged
} from './labels-view';

const HELD = ['Autumn term', 'Bundles', 'Phonics'];

describe('the rename rule', () => {
	it('accepts the trimmed name, which is the one the server stores', () => {
		expect(checkRename('  Spring term \n', 'Autumn term', HELD)).toEqual({
			accepted: true,
			name: 'Spring term'
		});
	});

	it('refuses a name that is only whitespace as empty', () => {
		for (const raw of ['', '   ', '\t\n']) {
			const verdict = checkRename(raw, 'Bundles', HELD);
			expect(verdict.accepted, `refused: ${JSON.stringify(raw)}`).toBe(false);
			expect(verdict.accepted === false && verdict.problem).toBe('empty');
		}
	});

	it('counts the bound in code points, after trimming', () => {
		const longest = 'é'.repeat(LABEL_MAX_CHARS);
		expect(checkRename(` ${longest} `, 'Bundles', HELD).accepted).toBe(true);
	});

	it('refuses one character past the bound, and says what the bound is', () => {
		const verdict = checkRename('e'.repeat(LABEL_MAX_CHARS + 1), 'Bundles', HELD);
		expect(verdict.accepted).toBe(false);
		expect(verdict.accepted === false && verdict.problem).toBe('too-long');
		expect(verdict.accepted === false && verdict.message).toContain(String(LABEL_MAX_CHARS));
	});

	it('refuses a slash, which the address path cannot carry', () => {
		const verdict = checkRename('Year 5/6', 'Bundles', HELD);
		expect(verdict.accepted).toBe(false);
		expect(verdict.accepted === false && verdict.problem).toBe('slash');
	});

	it('refuses a name another label already holds, whatever its case', () => {
		const verdict = checkRename('phonics', 'Bundles', HELD);
		expect(verdict.accepted).toBe(false);
		expect(verdict.accepted === false && verdict.problem).toBe('taken');
	});

	// The server trims before it matches, so a held name that arrived with
	// padding is the same name and the comparison has to trim both sides.
	it('refuses a name a padded held name already holds', () => {
		const verdict = checkRename('phonics', 'Bundles', ['  Phonics  ']);
		expect(verdict.accepted).toBe(false);
		expect(verdict.accepted === false && verdict.problem).toBe('taken');
	});

	// The unique index is over `lower(name)`, so recasing the label being
	// renamed updates the row it matched and conflicts with nothing.
	it('accepts recasing the label being renamed', () => {
		expect(checkRename('BUNDLES', 'Bundles', HELD).accepted).toBe(true);
	});
});

describe('an unchanged rename', () => {
	it('is unchanged once trimmed, and a recasing is not', () => {
		expect(unchanged('  Bundles  ', 'Bundles')).toBe(true);
		expect(unchanged('bundles', 'Bundles')).toBe(false);
	});
});

describe('the rows', () => {
	it('carries the count when one is known and null when it is not', () => {
		const built = rows(
			[
				{ name: 'Bundles', colour: 'teal', system: false },
				{ name: 'Phonics', colour: 'pink', system: false }
			],
			new Map([['Bundles', 12]])
		);
		expect(built).toEqual([
			{ name: 'Bundles', colour: 'teal', count: 12, system: false },
			{ name: 'Phonics', colour: 'pink', count: null, system: false }
		]);
	});

	// The row is what the page reads to decide whether to offer a rename or a
	// delete at all: both routes answer 404 for an import's own mark, so a
	// row that lost the flag would offer two controls whose refusal says the
	// label is not there.
	it('keeps an import’s own mark marked as one', () => {
		const built = rows([{ name: 'TPT', colour: 'green', system: true }], new Map());
		expect(built[0]).toEqual({
			name: 'TPT',
			colour: 'green',
			count: null,
			system: true
		});
	});
});

describe('the search', () => {
	const all = rows(
		[
			{ name: 'Autumn term', colour: 'amber', system: false },
			{ name: 'Bundles', colour: 'teal', system: false }
		],
		new Map()
	);

	it('shows everything for a blank term', () => {
		expect(matching(all, '   ').map((row) => row.name)).toEqual(['Autumn term', 'Bundles']);
	});

	it('matches part of a name, whatever its case', () => {
		expect(matching(all, ' TERM ').map((row) => row.name)).toEqual(['Autumn term']);
	});

	it('answers nothing when nothing matches', () => {
		expect(matching(all, 'phonics')).toEqual([]);
	});
});

describe('the meta line', () => {
	it('names the figure, singular at one', () => {
		expect(countLine(12, false)).toBe('12 resources');
		expect(countLine(1, false)).toBe('1 resource');
	});

	it('says a count is still being walked rather than showing a zero', () => {
		expect(countLine(null, true)).toBe('Counting…');
	});

	it('says a count could not be read rather than showing a zero', () => {
		expect(countLine(null, false)).toBe('Count unavailable');
	});
});

describe('the delete warning', () => {
	// The label is named because the panel opens in a list of rows that look
	// alike: a sentence carrying only a figure leaves the seller working out
	// which row it belongs to before destroying something.
	it('names the label, the count, and what deleting does to the resources', () => {
		expect(deleteWarning('Autumn term', 12)).toBe(
			'Autumn term is on 12 resources. Deleting it removes it from all of them.'
		);
		expect(deleteWarning('Phonics', 1)).toBe(
			'Phonics is on 1 resource. Deleting it removes it from that resource.'
		);
	});

	it('states the consequence without a figure when the count is not known', () => {
		expect(deleteWarning('Bundles', null)).toBe(
			'Deleting Bundles removes it from every resource that carries it.'
		);
	});
});

describe('the eyebrow', () => {
	it('counts what the list shows', () => {
		expect(totalLine(3)).toBe('Total labels: 3');
	});
});

describe('the swatch', () => {
	it('maps every colour of the generated set', () => {
		expect(swatchClass('violet')).toBe('c-violet');
		expect(swatchClass('slate')).toBe('c-slate');
	});

	it('falls back rather than rendering an unstyled swatch', () => {
		expect(swatchClass('chartreuse')).toBe('c-slate');
	});
});

describe('the filter link', () => {
	it('escapes the name, which travels as a query value', () => {
		expect(filterHref('Year 5 & 6')).toBe('/resources?label=Year%205%20%26%206');
	});
});

describe('counting every label', () => {
	it('counts each one and keys the answer by name', async () => {
		const { counts, failed } = await countAll(['a', 'b', 'c'], async (name) => name.length + 1);
		expect([...counts]).toEqual([
			['a', 2],
			['b', 2],
			['c', 2]
		]);
		expect(failed).toEqual([]);
	});

	it('runs no more walks at once than its width', async () => {
		let running = 0;
		let widest = 0;
		const names = Array.from({ length: 9 }, (_, index) => `label-${index}`);
		await countAll(
			names,
			async () => {
				running += 1;
				widest = Math.max(widest, running);
				await Promise.resolve();
				running -= 1;
				return 1;
			},
			3
		);
		expect(widest).toBe(3);
	});

	it('answers an empty pass for an empty vocabulary', async () => {
		expect(await countAll([], async () => 1)).toEqual({
			counts: new Map(),
			failed: []
		});
	});

	// One flaky walk costs its own label a figure and nothing else. Rejecting the
	// whole pass threw away every count that had landed and had the query retry
	// the entire pass, which is several walks of the catalogue for one failure.
	it('keeps the counts that landed and names the labels that failed', async () => {
		const { counts, failed } = await countAll(['a', 'b', 'c'], async (name) => {
			if (name === 'b') {
				throw new Error('the catalogue could not be read');
			}
			return 4;
		});
		expect([...counts]).toEqual([
			['a', 4],
			['c', 4]
		]);
		expect(failed).toEqual(['b']);
	});

	it('names every label when nothing could be counted', async () => {
		const { counts, failed } = await countAll(['a', 'b'], async () => {
			throw new Error('down');
		});
		expect(counts.size).toBe(0);
		expect(failed.sort()).toEqual(['a', 'b']);
	});
});
