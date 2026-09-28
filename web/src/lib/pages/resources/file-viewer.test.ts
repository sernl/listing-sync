import { describe, expect, it } from 'vitest';
import type { FileView } from '$lib/api';
import {
	DEVICE_ONLY,
	STORAGE_FULL,
	currentPage,
	keptPdfSource,
	pageWindow,
	sourceOfKept,
	storedPdfSource,
	viewFrom,
	viewerMode
} from './file-viewer';

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
	it('names the first PDF payload this device keeps, and nothing else', () => {
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
		await expect(gone.bytes()).rejects.toThrow('not kept on this device');
	});
});

describe('where View and the preview maker read a file from', () => {
	const product = 'p1';

	it('streams Teachouse’s copy wherever there is one, even inside the app', () => {
		const from = viewFrom(product, file({ server_copy: 'stored' }), null, true);
		expect(from.kind).toBe('server');
		expect(from.kind === 'server' && from.source.url).toBe('/v1/products/p1/files/f1/content');
	});

	it('reads a file older servers never labelled as Teachouse’s own', () => {
		expect(viewFrom(product, file({}), null, false).kind).toBe('server');
	});

	it('falls back to this device’s kept copy, and otherwise says the file is on the device', () => {
		const imported = file({ server_copy: 'device_only' });
		expect(viewFrom(product, imported, null, true).kind).toBe('device');
		expect(viewFrom(product, imported, null, false)).toEqual({
			kind: 'unavailable',
			sentence: DEVICE_ONLY
		});
		expect(viewFrom(product, file({ server_copy: 'storage_full' }), null, false)).toEqual({
			kind: 'unavailable',
			sentence: STORAGE_FULL
		});
	});

	it('cuts a preview from the stored PDF only once Teachouse holds it', async () => {
		const copied = storedPdfSource(product, [file({ server_copy: 'stored' })]);
		expect(copied?.unavailable).toBeUndefined();
		expect(copied?.url).toBe('/v1/products/p1/files/f1/content');
		const waiting = storedPdfSource(product, [file({ server_copy: 'device_only' })]);
		expect(waiting?.unavailable).toBe(DEVICE_ONLY);
		await expect(waiting?.bytes()).rejects.toThrow(DEVICE_ONLY);
		expect(storedPdfSource(product, [file({ kind: 'zip' })])).toBeNull();
	});
});
