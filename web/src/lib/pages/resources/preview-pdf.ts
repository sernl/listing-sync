import { PDFDocument, StandardFonts, degrees, rgb } from 'pdf-lib';

/** The brand indigo, `--primary` `#1E2A5A`, in pdf-lib's 0-to-1 components. */
const INDIGO = rgb(0.118, 0.165, 0.353);

/** Low enough that the page reads through it, high enough to deter a reseller. */
const WATERMARK_OPACITY = 0.18;

/** The watermark spans this much of the page's diagonal. */
const DIAGONAL_SPAN = 0.7;

const WATERMARK_ANGLE = 45;

/** The pages the teacher ticked, copied into a new document, optionally with
 *  the seller's name written diagonally across each one.
 *
 *  `pages` are 1-based, in the order the new document should carry them. */
export async function buildPreview(
	bytes: ArrayBuffer,
	pages: number[],
	watermark: string | null
): Promise<Uint8Array> {
	const source = await PDFDocument.load(bytes);
	const preview = await PDFDocument.create();
	const copied = await preview.copyPages(
		source,
		pages.map((page) => page - 1)
	);
	for (const page of copied) {
		preview.addPage(page);
	}

	if (watermark !== null && watermark.trim() !== '') {
		const font = await preview.embedFont(StandardFonts.Helvetica);
		const text = watermark.trim();
		const radians = (WATERMARK_ANGLE * Math.PI) / 180;
		const along = Math.cos(radians);
		const up = Math.sin(radians);
		for (const page of preview.getPages()) {
			const { width, height } = page.getSize();
			const diagonal = Math.hypot(width, height);
			// One unit of font size is one unit of the string's width at size 1,
			// so the size that makes the string span the target width is that
			// target divided by the unit width.
			const size = (diagonal * DIAGONAL_SPAN) / font.widthOfTextAtSize(text, 1);
			const drawnWidth = font.widthOfTextAtSize(text, size);
			const drawnHeight = font.heightAtSize(size);
			page.drawText(text, {
				// The baseline start, walked back half the string along the
				// rotation and half its height across it, so the string's middle
				// lands on the page's middle.
				x: width / 2 - (drawnWidth / 2) * along + (drawnHeight / 2) * up,
				y: height / 2 - (drawnWidth / 2) * up - (drawnHeight / 2) * along,
				size,
				font,
				color: INDIGO,
				opacity: WATERMARK_OPACITY,
				rotate: degrees(WATERMARK_ANGLE)
			});
		}
	}

	return preview.save();
}

/** The name the preview is uploaded under: the source's, marked as a preview. */
export function previewFileName(sourceName: string): string {
	const stem = sourceName.replace(/\.pdf$/i, '');
	return `${stem}-preview.pdf`;
}
