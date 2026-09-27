/**
 * The read-only viewer over a resource's files, and the source a preview can
 * be cut from: what to draw for a content type, and which pages to have drawn
 * for where the reader is.
 *
 * Pure, so the page-window arithmetic tests without a canvas.
 */

import { api, type FileView } from '$lib/api';
import type { Invoke } from '$lib/desktop';
import { libraryRead } from '$lib/desktop';

/** A file a preview can be cut from, and a viewer can show: a name, and a
 *  way to get its bytes when the dialog opens. */
export interface ByteSource {
	name: string;
	bytes: () => Promise<ArrayBuffer>;
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
				throw new Error('that file is not kept on this machine');
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
		(candidate) => candidate.role === 'payload' && candidate.kind === 'pdf' && kept.has(candidate.hash)
	);
	if (file === undefined) {
		return null;
	}
	return sourceOfKept(invoke, file.name ?? 'file.pdf', file.hash);
}

/** A file Teachouse stores for this resource, read back from the server
 *  when asked, never earlier. What a reopened resource has on any machine,
 *  in any browser. */
export function sourceOfStored(product: string, file: FileView): ByteSource {
	return {
		name: file.name ?? `file.${file.kind}`,
		bytes: () => api.productFileBytes(product, file.id)
	};
}

/** The stored PDF a preview is cut from on a saved resource: the first PDF
 *  payload, which is the file the thumbnail is drawn from too. */
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
