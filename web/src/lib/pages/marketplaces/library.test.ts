import { describe, expect, it } from 'vitest';
import type { LibraryFileView } from '$lib/api';
import type { LibraryEntry } from '$lib/desktop';
import {
	BROWSER_SENTENCE,
	elsewhereRows,
	formatBytes,
	holdsSentence,
	libraryRows,
	removePrompt,
	transferLabel,
	transferSentence,
	usageLine
} from './library';

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

describe('sizes as the seller reads them', () => {
	it('names the unit and rounds to one decimal above kilobytes', () => {
		expect(formatBytes(0)).toBe('0 B');
		expect(formatBytes(1_023)).toBe('1023 B');
		expect(formatBytes(2_048)).toBe('2.0 KB');
		expect(formatBytes(31_457_280)).toBe('30.0 MB');
		expect(formatBytes(3 * 1024 ** 3)).toBe('3.0 GB');
	});
});

describe('the usage line', () => {
	it('says when nothing is kept, and counts files and bytes otherwise', () => {
		expect(usageLine([], 0)).toBe('No files are kept on this machine yet.');
		expect(usageLine([entry()], 2_048)).toBe('1 file, 2.0 KB on this machine.');
		expect(usageLine([entry(), entry({ hash: 'cd'.repeat(32) })], 4_096)).toBe(
			'2 files, 4.0 KB on this machine.'
		);
	});
});

describe('the rows', () => {
	it('lists newest first and names the marketplace as the cards do', () => {
		const rows = libraryRows([
			entry({ hash: 'a'.repeat(64), kept_at: 1_000, marketplace: 'Tes' }),
			entry({ hash: 'b'.repeat(64), kept_at: 3_000 }),
			entry({ hash: 'c'.repeat(64), kept_at: 2_000 })
		]);
		expect(rows.map((row) => row.hash[0])).toEqual(['b', 'c', 'a']);
		expect(rows[2].marketplaceName).toBe('TES');
		expect(rows[0].size).toBe('2.0 KB');
	});
});

describe('the words', () => {
	it('tells a browser where files live and confirms a removal without touching the listing', () => {
		expect(BROWSER_SENTENCE).toBe('Files are kept on the machines running the Teachouse app.');
		expect(removePrompt('worksheet.pdf')).toBe(
			'Remove "worksheet.pdf" from this machine? Your listing and the marketplace copy are untouched.'
		);
	});
});

describe('getting a file onto this machine', () => {
	const files: LibraryFileView[] = [
		{
			hash: 'a'.repeat(64),
			file_name: 'a.pdf',
			byte_len: 2_048,
			holders: [{ device: 'phone', name: 'Pixel', online: false }],
			wanted_by: []
		},
		{
			hash: 'b'.repeat(64),
			file_name: 'b.pdf',
			byte_len: 4_096,
			holders: [
				{ device: 'phone', name: 'Pixel', online: true },
				{ device: 'laptop', name: 'founder-pc', online: true }
			],
			wanted_by: ['laptop']
		},
		{
			hash: 'c'.repeat(64),
			file_name: null,
			byte_len: 1,
			holders: [{ device: 'laptop', name: 'founder-pc', online: true }],
			wanted_by: []
		}
	];

	it('offers Get where another machine holds it, names the holder, and says nothing when held here', () => {
		expect(transferLabel('a'.repeat(64), 'laptop', files, new Set())).toEqual({ kind: 'get', from: 'Pixel' });
		expect(transferLabel('a'.repeat(64), 'laptop', files, new Set(['a'.repeat(64)]))).toEqual({ kind: 'held' });
		expect(transferLabel('c'.repeat(64), 'laptop', files, new Set())).toEqual({ kind: 'nowhere' });
	});

	it('waits on an offline holder and copies from an online one once asked', () => {
		const asked = files.map((file) =>
			file.hash === 'a'.repeat(64) ? { ...file, wanted_by: ['laptop'] } : file
		);
		expect(transferLabel('a'.repeat(64), 'laptop', asked, new Set())).toEqual({ kind: 'waiting', on: 'Pixel' });
		expect(transferSentence({ kind: 'waiting', on: 'Pixel' })).toBe('Waiting for Pixel.');
		expect(transferLabel('b'.repeat(64), 'laptop', files, new Set())).toEqual({ kind: 'fetching', from: 'Pixel' });
	});

	it('lists what is elsewhere, by name, and skips what only this machine holds', () => {
		const rows = elsewhereRows(files, 'laptop', new Set());
		expect(rows.map((row) => row.name)).toEqual(['a.pdf', 'b.pdf']);
		expect(rows[0].size).toBe('2.0 KB');
	});

	it('counts what a machine holds for its row', () => {
		expect(holdsSentence('phone', files)).toBe('holds 2 files');
		expect(holdsSentence('laptop', files)).toBe('holds 2 files');
		expect(holdsSentence('tablet', files)).toBeNull();
		expect(holdsSentence('laptop', files.slice(2))).toBe('holds 1 file');
	});
});
