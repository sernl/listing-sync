/**
 * The pixel size a file under `public/` draws at, read from its own header at
 * build time, so an `<img>` can carry `width` and `height` and the browser
 * reserves its box before the bytes arrive. The stylesheet still decides the
 * drawn size (a height and `width: auto`); the attributes only give it the
 * ratio. SVG by its `viewBox` (or `width`/`height`), PNG by its `IHDR`, JPEG
 * by its first frame header; anything else answers nothing and the tag is
 * written without the pair.
 */
import { readFileSync } from 'node:fs';

/* The build runs from `apps/landing` (`npm run build`, locally and in
   `nix/landing.nix`); the bundled page code no longer sits beside `public/`,
   so `import.meta.url` cannot find it. */
const publicDir = `${process.cwd()}/public/`;

const svgSize = (text) => {
	const open = /<svg\b[^>]*>/.exec(text)?.[0] ?? '';
	const box = /viewBox="\s*[-\d.]+[\s,]+[-\d.]+[\s,]+([\d.]+)[\s,]+([\d.]+)\s*"/.exec(open);
	if (box) return { width: Math.round(Number(box[1])), height: Math.round(Number(box[2])) };
	const width = /\bwidth="([\d.]+)(?:px)?"/.exec(open);
	const height = /\bheight="([\d.]+)(?:px)?"/.exec(open);
	return width && height
		? { width: Math.round(Number(width[1])), height: Math.round(Number(height[1])) }
		: null;
};

const pngSize = (bytes) =>
	bytes.toString('ascii', 12, 16) === 'IHDR'
		? { width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20) }
		: null;

const jpegSize = (bytes) => {
	let at = 2;
	while (at + 9 < bytes.length && bytes[at] === 0xff) {
		const marker = bytes[at + 1];
		const length = bytes.readUInt16BE(at + 2);
		// SOF0..SOF15, except DHT (C4), JPG (C8) and DAC (CC).
		if (marker >= 0xc0 && marker <= 0xcf && ![0xc4, 0xc8, 0xcc].includes(marker)) {
			return { width: bytes.readUInt16BE(at + 7), height: bytes.readUInt16BE(at + 5) };
		}
		at += 2 + length;
	}
	return null;
};

/** `{ width, height }` for `public/<path>`, or `{}`. */
export const intrinsic = (path) => {
	const bytes = readFileSync(publicDir + path);
	const size = path.endsWith('.svg')
		? svgSize(bytes.toString('utf8'))
		: path.endsWith('.png')
			? pngSize(bytes)
			: /\.jpe?g$/.test(path)
				? jpegSize(bytes)
				: null;
	return size ?? {};
};
