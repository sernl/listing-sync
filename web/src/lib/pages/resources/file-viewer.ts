/**
 * The read-only viewer over a resource's files, and the source a preview can
 * be cut from: what to draw for a content type, and which pages to have drawn
 * for where the reader is.
 *
 * Pure, so the page-window arithmetic tests without a canvas.
 */

import { ApiFailure, api, type FileView, type LibraryHolderView } from '$lib/api';
import type { Invoke } from '$lib/desktop';
import { libraryRead } from '$lib/desktop';

/** A file a preview can be cut from, and a viewer can show: a name, and a
 *  way to get its bytes when the dialog opens. */
export interface ByteSource {
	name: string;
	bytes: () => Promise<ArrayBuffer>;
	/** Where the viewer can stream it from instead, a page at a time. Only a
	 *  file read through Teachouse has one. */
	url?: string;
	/** Asks whether the bytes can be read now, before a control hands them
	 *  on. Present only for an imported file, which Teachouse passes on from
	 *  a device of the seller's and so can find unreachable; an upload and
	 *  this device's own kept copy are always there. */
	probe?: () => Promise<Reach>;
}

// ------------------------------------------------------- where a file is

/** Why an imported file can't be read right now: the device holding it is
 *  off, or the Teachouse app there is closed. */
export function offlineSentence(device: string): string {
	return `Your file is on ${device}, which is offline. Open the Teachouse app there.`;
}

/** Where no device of the seller's reports holding the file. */
export const NO_DEVICE_HAS_IT = 'None of your devices has this file any more.';

/** Where the server cannot pass files on at all just now. */
export const STREAMING_UNAVAILABLE =
	'Opening files from your devices isn’t available right now. Try again later.';

/** Where the probe itself did not arrive. */
export const PROBE_FAILED = 'Teachouse couldn’t reach your file. Try again.';

/** How an imported file reaches a browser, for the Explain beside the
 *  offline sentence. */
export const STREAM_EXPLAINED = [
	'Files you import stay on your devices. Teachouse passes them to your browser while that device is on and the Teachouse app is open.',
	'On Android, the app shows “Teachouse is sharing your files” while it can pass them on.',
	'Teachouse doesn’t keep a copy.',
	'Files you upload yourself are always available.'
];

/** What asking for a file's bytes found. `offline` carries the Explain;
 *  `refused` is a sentence on its own. */
export type Reach =
	{ kind: 'ready' } | { kind: 'offline'; device: string } | { kind: 'refused'; sentence: string };

/** A reach that is not `ready`, which is what a row shows. */
export type Unreachable = Exclude<Reach, { kind: 'ready' }>;

export function reachSentence(reach: Unreachable): string {
	return reach.kind === 'offline' ? offlineSentence(reach.device) : reach.sentence;
}

/** The device named in a `device_offline` refusal's detail. */
function detailDevice(detail: unknown): string | null {
	if (typeof detail === 'object' && detail !== null && 'device' in detail) {
		const device = detail.device;
		return typeof device === 'string' && device.trim() !== '' ? device : null;
	}
	return null;
}

/** What a refused probe means for the seller. For an offline device, the
 *  name is a holder the console already knows is online where there is
 *  one — the device the seller will most likely open the app on — and
 *  otherwise the device the server tried. */
export function reachOf(failure: unknown, holders: readonly LibraryHolderView[]): Unreachable {
	if (!(failure instanceof ApiFailure)) {
		return { kind: 'refused', sentence: PROBE_FAILED };
	}
	switch (failure.code()) {
		case 'device_offline':
			return {
				kind: 'offline',
				device:
					holders.find((holder) => holder.online)?.name ??
					detailDevice(failure.body?.errors[0]?.detail) ??
					holders[0]?.name ??
					'your device'
			};
		case 'resource_missing':
			return { kind: 'refused', sentence: NO_DEVICE_HAS_IT };
		case 'streaming_unavailable':
			return { kind: 'refused', sentence: STREAMING_UNAVAILABLE };
		default:
			return { kind: 'refused', sentence: failure.message };
	}
}

/** Asks the content URL whether the bytes can be read now. */
export async function probeReach(
	url: string,
	holders: readonly LibraryHolderView[]
): Promise<Reach> {
	try {
		await api.probeFile(url);
		return { kind: 'ready' };
	} catch (failure) {
		return reachOf(failure, holders);
	}
}

/** What View opens for one stored file, in order of preference: an upload
 *  through Teachouse, which streams a range at a time; this device's own
 *  kept copy, inside the app, which needs no other device; an imported file
 *  through Teachouse from a device that holds it, probed first; or nothing,
 *  where no device reports it. */
export type ViewFrom =
	| { kind: 'server'; source: ByteSource }
	| { kind: 'device'; source: ByteSource }
	| { kind: 'unavailable'; sentence: string };

export function viewFrom(
	product: string,
	file: FileView,
	invoke: Invoke | null,
	keptHere: boolean
): ViewFrom {
	if (file.custody.kind === 'uploaded') {
		return { kind: 'server', source: sourceOfStored(product, file) };
	}
	if (keptHere) {
		return { kind: 'device', source: sourceOfKept(invoke, file.name ?? 'file', file.hash) };
	}
	if (file.custody.holders.length === 0) {
		return { kind: 'unavailable', sentence: NO_DEVICE_HAS_IT };
	}
	return { kind: 'server', source: sourceOfStored(product, file) };
}

/** The in-session upload as a source. */
export function sourceOfFile(file: File): ByteSource {
	return { name: file.name, bytes: () => file.arrayBuffer() };
}

/** A file kept on this machine as a source. The bytes are read from the
 *  application when asked, never earlier, and a machine that no longer
 *  keeps the file answers by throwing, which the maker words. */
export function sourceOfKept(invoke: Invoke | null, name: string, hash: string): ByteSource {
	return {
		name,
		bytes: async () => {
			const bytes = await libraryRead(invoke, hash);
			if (bytes === null) {
				throw new Error('that file is not kept on this device');
			}
			return bytes;
		}
	};
}

/** The stored file a preview could be cut from, where this machine keeps
 *  it: the first PDF payload whose digest the library lists. */
export function keptPdfSource(
	invoke: Invoke | null,
	files: readonly FileView[],
	kept: ReadonlySet<string>
): ByteSource | null {
	const file = files.find(
		(candidate) =>
			candidate.role === 'payload' && candidate.kind === 'pdf' && kept.has(candidate.hash)
	);
	if (file === undefined) {
		return null;
	}
	return sourceOfKept(invoke, file.name ?? 'file.pdf', file.hash);
}

/** One of this resource's files read through Teachouse when asked, never
 *  earlier: an upload from Teachouse's store, an imported file passed on
 *  from a device that holds it — which is why only the second carries a
 *  probe. What a reopened resource has on any machine, in any browser. */
export function sourceOfStored(product: string, file: FileView): ByteSource {
	const url = api.productFileUrl(product, file.id);
	const custody = file.custody;
	return {
		name: file.name ?? `file.${file.kind}`,
		bytes: () => api.productFileBytes(product, file.id),
		url,
		...(custody.kind === 'devices' ? { probe: () => probeReach(url, custody.holders) } : {})
	};
}

/** The stored PDF a preview is cut from on a saved resource: the first PDF
 *  payload, which is the file the thumbnail is drawn from too. An imported
 *  one carries its probe, so the maker asks before it opens and the seller
 *  reads why rather than a maker that never fills. */
export function storedPdfSource(product: string, files: readonly FileView[]): ByteSource | null {
	const file = files.find((candidate) => candidate.role === 'payload' && candidate.kind === 'pdf');
	return file === undefined ? null : sourceOfStored(product, file);
}

/** The type a stored file is drawn as, from the kind the product view names. */
export function contentTypeOfKind(kind: FileView['kind']): string {
	switch (kind) {
		case 'pdf':
			return 'application/pdf';
		case 'image':
			return 'image/png';
		default:
			return 'application/octet-stream';
	}
}

/** How the viewer draws one content type. */
export type ViewerMode = 'pdf' | 'image' | 'other';

export function viewerMode(contentType: string): ViewerMode {
	if (contentType === 'application/pdf') {
		return 'pdf';
	}
	return contentType.startsWith('image/') ? 'image' : 'other';
}

/** The sentence the viewer shows for a type it does not draw. */
export const OPENS_ELSEWHERE = 'This file type opens in another app.';

/** The sentence when the phone cannot hand the file to another app. */
export const OPEN_UNAVAILABLE = 'Opening in another app is not available on this phone yet.';

/** How many pages either side of the current one are kept drawn. */
export const PAGE_MARGIN = 2;

/** The pages to have drawn, given the one the reader is on: the current
 *  page and two either side, clipped to the document. Pages outside the
 *  window are released, so a hundred-page document never holds a hundred
 *  canvases. */
export function pageWindow(current: number, pageCount: number): number[] {
	if (pageCount <= 0) {
		return [];
	}
	const first = Math.max(1, Math.min(current, pageCount) - PAGE_MARGIN);
	const last = Math.min(pageCount, Math.max(current, 1) + PAGE_MARGIN);
	return Array.from({ length: last - first + 1 }, (_, index) => first + index);
}

/** Which page is under the reader, from the page tops and the scroll
 *  position: the last page whose top is at or above the viewport's middle. */
export function currentPage(tops: readonly number[], scrollTop: number, viewport: number): number {
	const middle = scrollTop + viewport / 2;
	let current = 1;
	for (const [index, top] of tops.entries()) {
		if (top <= middle) {
			current = index + 1;
		}
	}
	return current;
}
