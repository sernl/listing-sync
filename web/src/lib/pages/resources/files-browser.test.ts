import { describe, expect, it } from 'vitest';
import type { LibraryFileView } from '$lib/api';
import type { LibraryEntry } from '$lib/desktop';
import {
	HERE,
	availabilityOf,
	countSentence,
	fileRows,
	filtersFromUrl,
	filtersToQuery,
	filtersToUrl,
	machineFilesHref,
	metaLine,
	pageWindow,
	transferLabel
} from './files-browser';

function entry(over: Partial<LibraryEntry> = {}): LibraryEntry {
	return {
		hash: 'ab'.repeat(32),
		file_name: 'worksheet.pdf',
		content_type: 'application/pdf',
		byte_len: 2_048,
		marketplace: 'Tpt',
		resource: '101',
		kept_at: 1_000,
		pinned: false,
		...over
	};
}

function file(over: Partial<LibraryFileView> = {}): LibraryFileView {
	return {
		hash: 'a'.repeat(64),
		file_name: 'a.pdf',
		byte_len: 2_048,
		holders: [{ device: 'phone', name: 'Pixel', online: false }],
		wanted_by: [],
		resources: [],
		...over
	};
}


describe('where a file is', () => {
	it('tells a live holder, an offline holder and no holder apart', () => {
		expect(availabilityOf(file({ holders: [{ device: 'p', name: 'Pixel', online: true }] }))).toBe(
			'online'
		);
		expect(availabilityOf(file())).toBe('offline');
		expect(availabilityOf(file({ holders: [] }))).toBe('missing');
	});
});

describe('getting a file onto this machine', () => {
	const offline = file();
	const wanted = file({
		hash: 'b'.repeat(64),
		holders: [
			{ device: 'phone', name: 'Pixel', online: true },
			{ device: 'laptop', name: 'founder-pc', online: true }
		],
		wanted_by: ['laptop']
	});

	it('offers Get where another machine holds it, and says nothing when held here', () => {
		expect(transferLabel(offline, 'laptop', false)).toEqual({ kind: 'get', from: 'Pixel' });
		expect(transferLabel(offline, 'laptop', true)).toEqual({ kind: 'held' });
		expect(transferLabel(file({ holders: [] }), 'laptop', false)).toEqual({ kind: 'missing' });
	});

	it('reads a holding of this machine as held even before its own library answers', () => {
		const here = file({ holders: [{ device: 'laptop', name: 'founder-pc', online: true }] });
		expect(transferLabel(here, 'laptop', null)).toEqual({ kind: 'held' });
	});

	it('waits on an offline holder and copies from an online one once asked', () => {
		expect(transferLabel({ ...offline, wanted_by: ['laptop'] }, 'laptop', false)).toEqual({
			kind: 'waiting',
			on: 'Pixel'
		});
		expect(transferLabel(wanted, 'laptop', false)).toEqual({ kind: 'fetching', from: 'Pixel' });
	});

	it('offers nothing in a browser, and states who holds the file instead', () => {
		const label = transferLabel(offline, null, false);
		expect(label).toEqual({ kind: 'elsewhere', on: 'Pixel' });
	});

	it('trusts a completed local read over an older self-holding report', () => {
		expect(transferLabel({ ...wanted, wanted_by: [] }, 'laptop', false)).toEqual({
			kind: 'get',
			from: 'Pixel'
		});
	});

	it('keeps a pending copy cancellable after every source disappears', () => {
		expect(transferLabel(file({ holders: [], wanted_by: ['laptop'] }), 'laptop', false)).toEqual({
			kind: 'waiting',
			on: null
		});
	});

	it('does not offer a transfer into an unavailable local library', () => {
		expect(transferLabel(offline, 'laptop', null)).toEqual({
			kind: 'elsewhere',
			on: 'Pixel'
		});
	});
});

describe('the rows', () => {
	it('keeps the order the server paged in, and names an unnamed file by its digest', () => {
		const rows = fileRows(
			[
				file({ hash: 'b'.repeat(64), file_name: 'b.pdf' }),
				file({ hash: 'c'.repeat(64), file_name: null })
			],
			{ thisDevice: 'laptop', kept: new Map() }
		);
		expect(rows.map((row) => row.name)).toEqual(['b.pdf', `${'c'.repeat(12)}…`]);
		expect(rows[1].anonymous).toBe(true);
	});

	it('names this machine by its role rather than by the name it registered under', () => {
		const rows = fileRows(
			[
				file({
					holders: [
						{ device: 'laptop', name: 'founder-pc', online: true },
						{ device: 'phone', name: 'Pixel', online: false }
					]
				})
			],
			{ thisDevice: 'laptop', kept: new Map() }
		);
		expect(rows[0].holders).toBe(`On ${HERE}, Pixel`);
		expect(rows[0].holders).not.toContain('founder-pc');
	});

	it('shows a kept date only for a file this machine actually keeps', () => {
		const held = fileRows([file()], {
			thisDevice: 'laptop',
			kept: new Map([['a'.repeat(64), entry({ hash: 'a'.repeat(64), kept_at: 1_000 })]])
		});
		expect(metaLine(held[0])).toContain('kept');
		const remote = fileRows([file()], { thisDevice: 'laptop', kept: new Map() });
		expect(metaLine(remote[0])).not.toContain('kept');
	});

	it('carries every resource the file belongs to, and says so when none does', () => {
		const rows = fileRows(
			[
				file({
					resources: [
						{ id: '11111111-1111-4111-8111-111111111111', title: 'Fractions' },
						{ id: '22222222-2222-4222-8222-222222222222', title: 'Decimals' }
					]
				}),
				file({ hash: 'd'.repeat(64) })
			],
			{ thisDevice: 'laptop', kept: new Map() }
		);
		expect(rows[0].resources.map((one) => one.title)).toEqual(['Fractions', 'Decimals']);
		expect(metaLine(rows[0])).toContain('2 resources');
		expect(metaLine(rows[1])).toContain('No resource uses it');
	});
});

describe('the filters in the address', () => {
	it('reads what it wrote, and writes nothing for a bare browser', () => {
		const filters = filtersFromUrl(
			new URLSearchParams('q=frac&device=laptop&availability=offline&linked=unlinked&offset=50')
		);
		expect(filters).toEqual({
			q: 'frac',
			device: 'laptop',
			availability: 'offline',
			linked: 'unlinked',
			offset: 50
		});
		expect(filtersToUrl(filters)).toBe(
			'/resources/files?q=frac&device=laptop&availability=offline&linked=unlinked&offset=50'
		);
		expect(filtersToUrl(filtersFromUrl(new URLSearchParams()))).toBe('/resources/files');
	});

	it('drops a filter nobody could have chosen rather than narrowing to nothing', () => {
		const filters = filtersFromUrl(
			new URLSearchParams('availability=somewhere&linked=maybe&offset=-3')
		);
		expect(filters.availability).toBeNull();
		expect(filters.linked).toBeNull();
		expect(filters.offset).toBe(0);
	});

	it('asks the server only for what was chosen, and always for one page', () => {
		expect(filtersToQuery(filtersFromUrl(new URLSearchParams()))).toEqual({
			offset: 0,
			limit: 25
		});
		expect(filtersToQuery(filtersFromUrl(new URLSearchParams('device=phone&q=+frac+')))).toEqual({
			q: 'frac',
			device: 'phone',
			offset: 0,
			limit: 25
		});
	});

	it('links one machine to its own files', () => {
		expect(machineFilesHref('laptop')).toBe('/resources/files?device=laptop');
	});
});

describe('the pager', () => {
	it('counts the window from what the server served rather than from what was asked', () => {
		const first = pageWindow(25, 60, 0, 25);
		expect(countSentence(first)).toBe('Showing 1–25 of 60 files');
		expect(first.hasPrev).toBe(false);
		expect(first.hasNext).toBe(true);
		expect(first.nextOffset).toBe(25);

		const last = pageWindow(10, 60, 50, 25);
		expect(countSentence(last)).toBe('Showing 51–60 of 60 files');
		expect(last.hasNext).toBe(false);
		expect(last.prevOffset).toBe(25);
	});

	it('offers a way back from a page past the end, and claims no rows there', () => {
		const past = pageWindow(0, 60, 100, 25);
		expect(past.from).toBe(0);
		expect(past.hasNext).toBe(false);
		expect(past.hasPrev).toBe(true);
		expect(past.prevOffset).toBe(75);
	});

	it('says there are none rather than showing a range of nothing', () => {
		expect(countSentence(pageWindow(0, 0, 0, 25))).toBe('No files');
	});
});
