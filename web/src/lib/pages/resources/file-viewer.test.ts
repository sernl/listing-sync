import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiFailure, type FileView, type LibraryHolderView } from '$lib/api';
import type { APIErrorCode } from '$lib/generated/vocab';
import {
	NO_DEVICE_HAS_IT,
	STREAMING_UNAVAILABLE,
	currentPage,
	keptPdfSource,
	pageWindow,
	reachOf,
	reachSentence,
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
		custody: { kind: 'uploaded' },
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

const laptop: LibraryHolderView = { device: 'd1', name: 'Laptop', online: false };
const phone: LibraryHolderView = { device: 'd2', name: 'Pixel', online: true };

function refusal(status: number, code: APIErrorCode, detail?: unknown): ApiFailure {
	return new ApiFailure(status, {
		status,
		errors: [{ code, message: `server said ${code}`, detail }]
	});
}

describe('where View and the preview maker read a file from', () => {
	const product = 'p1';
	const imported = (holders: LibraryHolderView[]) =>
		file({ custody: { kind: 'devices', holders } });

	afterEach(() => vi.unstubAllGlobals());

	it('reads an upload through Teachouse with no probe, even inside the app', () => {
		const from = viewFrom(product, file({}), null, true);
		expect(from.kind).toBe('server');
		expect(from.kind === 'server' && from.source.url).toBe('/v1/products/p1/files/f1/content');
		expect(from.kind === 'server' && from.source.probe).toBeUndefined();
	});

	it('prefers this device’s own copy of an imported file over asking another device', () => {
		expect(viewFrom(product, imported([laptop]), null, true).kind).toBe('device');
	});

	it('streams an imported file from its devices, probing first', () => {
		const from = viewFrom(product, imported([laptop]), null, false);
		expect(from.kind).toBe('server');
		expect(from.kind === 'server' && from.source.url).toBe('/v1/products/p1/files/f1/content');
		expect(from.kind === 'server' && typeof from.source.probe).toBe('function');
	});

	it('says no device has an imported file nothing reports', () => {
		expect(viewFrom(product, imported([]), null, false)).toEqual({
			kind: 'unavailable',
			sentence: NO_DEVICE_HAS_IT
		});
	});

	it('probes the content URL and passes a ready answer through', async () => {
		const seen: string[] = [];
		vi.stubGlobal(
			'fetch',
			vi.fn(async (url: string) => {
				seen.push(url);
				return new Response(null, { status: 204 });
			})
		);
		const source = storedPdfSource(product, [imported([laptop])]);
		expect(await source?.probe?.()).toEqual({ kind: 'ready' });
		expect(seen).toEqual(['/v1/products/p1/files/f1/content?probe=1']);
	});

	it('cuts a preview from the first PDF payload, uploaded or imported', () => {
		expect(storedPdfSource(product, [file({})])?.probe).toBeUndefined();
		expect(storedPdfSource(product, [imported([laptop])])?.url).toBe(
			'/v1/products/p1/files/f1/content'
		);
		expect(storedPdfSource(product, [file({ kind: 'zip' })])).toBeNull();
	});
});

describe('what a refused probe says', () => {
	it('names an online holder over the device the server tried', () => {
		const reach = reachOf(refusal(409, 'device_offline', { device: 'Laptop' }), [phone, laptop]);
		expect(reachSentence(reach)).toBe(
			'Your file is on Pixel, which is offline. Open the Teachouse app there.'
		);
		expect(reach.kind).toBe('offline');
	});

	it('falls back to the server’s device, then to the first holder', () => {
		expect(reachOf(refusal(409, 'device_offline', { device: 'Laptop' }), [])).toEqual({
			kind: 'offline',
			device: 'Laptop'
		});
		expect(reachOf(refusal(409, 'device_offline'), [laptop])).toEqual({
			kind: 'offline',
			device: 'Laptop'
		});
	});

	it('words a missing file and a broker that is down without the Explain', () => {
		expect(reachOf(refusal(404, 'resource_missing'), [laptop])).toEqual({
			kind: 'refused',
			sentence: NO_DEVICE_HAS_IT
		});
		expect(reachOf(refusal(503, 'streaming_unavailable'), [laptop])).toEqual({
			kind: 'refused',
			sentence: STREAMING_UNAVAILABLE
		});
	});

	it('keeps the server’s own words for anything else, and says so when nothing answered', () => {
		expect(reachOf(refusal(401, 'session_required'), [])).toEqual({
			kind: 'refused',
			sentence: 'server said session_required'
		});
		expect(reachOf(new TypeError('network'), []).kind).toBe('refused');
	});
});
