import { describe, expect, it } from 'vitest';
import type { FileView } from '$lib/api';
import { currentPage, keptPdfSource, pageWindow, sourceOfKept, viewerMode } from './file-viewer';

function file(over: Partial<FileView>): FileView {
	return {
		id: 'f1',
		role: 'payload',
		kind: 'pdf',
		byte_len: 10,
		hash: 'a'.repeat(64),
		scan: 'clean',
		name: 'a.pdf',
		...over
	};
}

describe('the page window', () => {
	it('keeps the current page and two either side, clipped to the document', () => {
		expect(pageWindow(1, 10)).toEqual([1, 2, 3]);
		expect(pageWindow(5, 10)).toEqual([3, 4, 5, 6, 7]);
		expect(pageWindow(10, 10)).toEqual([8, 9, 10]);
		expect(pageWindow(3, 3)).toEqual([1, 2, 3]);
		expect(pageWindow(1, 0)).toEqual([]);
	});

	it('clamps a page outside the document', () => {
		expect(pageWindow(50, 4)).toEqual([2, 3, 4]);
		expect(pageWindow(-3, 4)).toEqual([1, 2, 3]);
	});
});

describe('the page under the reader', () => {
	it('is the last page whose top is above the middle of the viewport', () => {
		const tops = [0, 800, 1600, 2400];
		expect(currentPage(tops, 0, 600)).toBe(1);
		expect(currentPage(tops, 700, 600)).toBe(2);
		expect(currentPage(tops, 1900, 600)).toBe(3);
		expect(currentPage(tops, 5000, 600)).toBe(4);
		expect(currentPage([], 0, 600)).toBe(1);
	});
});

describe('what the viewer draws', () => {
	it('draws PDFs and images and defers everything else', () => {
		expect(viewerMode('application/pdf')).toBe('pdf');
		expect(viewerMode('image/png')).toBe('image');
		expect(viewerMode('application/zip')).toBe('other');
	});
});

describe('the kept source', () => {
	it('names the first PDF payload this machine keeps, and nothing else', () => {
		const kept = new Set(['b'.repeat(64)]);
		const files = [
			file({ id: 'cover', role: 'cover', kind: 'image', hash: 'b'.repeat(64) }),
			file({ id: 'zip', kind: 'zip', hash: 'b'.repeat(64), name: 'pack.zip' }),
			file({ id: 'gone', hash: 'c'.repeat(64) }),
			file({ id: 'here', hash: 'b'.repeat(64), name: 'kept.pdf' })
		];
		expect(keptPdfSource(null, files, kept)?.name).toBe('kept.pdf');
		expect(keptPdfSource(null, files, new Set())).toBeNull();
	});

	it('reads the bytes from the application when asked and refuses when they are gone', async () => {
		const bytes = new Uint8Array([1, 2, 3]);
		const invoke = async (command: string) => (command === 'library_read' ? bytes : null);
		const source = sourceOfKept(invoke, 'a.pdf', 'a'.repeat(64));
		expect(new Uint8Array(await source.bytes())).toEqual(bytes);
		const gone = sourceOfKept(null, 'a.pdf', 'a'.repeat(64));
		await expect(gone.bytes()).rejects.toThrow('not kept on this machine');
	});
});
