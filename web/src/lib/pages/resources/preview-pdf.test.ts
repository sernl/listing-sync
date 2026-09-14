import { describe, expect, it } from 'vitest';
import { PDFDocument, PDFRawStream, decodePDFRawStream } from 'pdf-lib';
import { buildPreview, previewFileName } from './preview-pdf';

const SIZES: [number, number][] = [
	[612, 792],
	[400, 400],
	[595, 842]
];

/** A three-page document whose pages have distinct sizes, so a copied page can
 *  be told from the one beside it. */
async function source(): Promise<ArrayBuffer> {
	const document = await PDFDocument.create();
	for (const [width, height] of SIZES) {
		document.addPage([width, height]);
	}
	const bytes = await document.save();
	return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer;
}

/** Every content stream in the document, inflated, as one string: pdf-lib
 *  compresses what it draws, so the raw file never spells the watermark out. */
async function drawn(bytes: Uint8Array): Promise<string> {
	const document = await PDFDocument.load(bytes);
	const decoder = new TextDecoder('latin1');
	return document.context
		.enumerateIndirectObjects()
		.map(([, object]) =>
			object instanceof PDFRawStream ? decoder.decode(decodePDFRawStream(object).decode()) : ''
		)
		.join('\n');
}

/** The way a PDF content stream spells a string pdf-lib has drawn. */
function shown(text: string): RegExp {
	const hex = [...text]
		.map((letter) => letter.charCodeAt(0).toString(16).padStart(2, '0').toUpperCase())
		.join('');
	return new RegExp(`<${hex}> Tj`, 'g');
}

describe('buildPreview', () => {
	it('copies the ticked pages, in order, at their own sizes', async () => {
		const made = await PDFDocument.load(await buildPreview(await source(), [1, 3], null));

		expect(made.getPageCount()).toBe(2);
		expect(made.getPage(0).getSize()).toMatchObject({ width: 612, height: 792 });
		expect(made.getPage(1).getSize()).toMatchObject({ width: 595, height: 842 });
	});

	it('writes the name across every copied page, and writes nothing without one', async () => {
		const bytes = await source();
		const plain = await buildPreview(bytes, [1, 3], null);
		const marked = await buildPreview(bytes, [1, 3], 'Jane Teacher');

		// One show-text operator per copied page.
		expect((await drawn(marked)).match(shown('Jane Teacher'))).toHaveLength(2);
		expect(marked.byteLength).toBeGreaterThan(plain.byteLength);
		expect(await drawn(plain)).not.toContain('Tj');
	});

	it('writes the name only across the pages chosen for it', async () => {
		const bytes = await source();
		const partly = await buildPreview(bytes, [1, 2, 3], 'Jane Teacher', [1, 3]);

		expect((await drawn(partly)).match(shown('Jane Teacher'))).toHaveLength(2);
		const made = await PDFDocument.load(partly);
		expect(made.getPageCount()).toBe(3);
		// A marked page not among the copied ones marks nothing.
		const none = await buildPreview(bytes, [2], 'Jane Teacher', [1, 3]);
		expect(await drawn(none)).not.toContain('Tj');
	});

	it('writes the name diagonally', async () => {
		const marked = await buildPreview(await source(), [1], 'Jane Teacher');

		const matrix = /(-?[\d.]+) (-?[\d.]+) (-?[\d.]+) (-?[\d.]+) [\d.]+ [\d.]+ Tm/.exec(
			await drawn(marked)
		);
		expect(matrix).not.toBeNull();
		const [a, b] = [Number(matrix?.[1]), Number(matrix?.[2])];
		expect((Math.atan2(b, a) * 180) / Math.PI).toBeCloseTo(45, 3);
	});

	it('treats a blank name as no watermark', async () => {
		const blank = await buildPreview(await source(), [2], '   ');

		expect(await drawn(blank)).not.toContain('Tj');
	});
});

describe('previewFileName', () => {
	it('marks the source name as a preview', () => {
		expect(previewFileName('a.pdf')).toBe('a-preview.pdf');
		expect(previewFileName('a')).toBe('a-preview.pdf');
		expect(previewFileName('Fractions Pack.PDF')).toBe('Fractions Pack-preview.pdf');
	});
});
