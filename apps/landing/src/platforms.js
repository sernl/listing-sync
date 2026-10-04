/**
 * The platforms this site says a build exists for, what each may draw, and
 * where each links.
 *
 * The row exists because the sentence beside it tells a reader that some work
 * needs a small app, and a reader on a phone cannot tell from that sentence
 * whether their phone is one of them. Each chip links to that platform's
 * download: `PlatformRow.astro` reads the download mirror's
 * `/downloads/downloads.json` on this same origin and points the chip at the
 * file, or at the store listing where there is no file, and a chip with
 * nothing published says so. Without the script, or before the first release
 * of a platform, every chip links to the console's download cards, which say
 * the same thing for a signed-in seller.
 *
 * `key` is the download manifest's own key for the platform, so the two cannot
 * name a platform differently.
 *
 * A row carries a file only where the platform's owner permits us to draw its
 * mark, so `file` is null far more often than it is missing. A row without one
 * names a generic `glyph` instead (a desktop screen), which is ours and no
 * vendor's, so the chips read at the same weight. Google licenses the Android
 * robot under Creative Commons with an attribution line this row carries;
 * Microsoft requires an express licence for the Windows symbol, which the
 * founder decided on 2026-09-07 not to apply for -- the drafted request stays
 * unsent in `docs/notes/design/marketplace-logo-sources.md` -- and permits the
 * name in text. The mark files are copies under `public/vendors/` rather than
 * links to the console's, for the reason `public/marks/` holds copies.
 *
 * The Android name carries its trademark symbol because Google's brand
 * guidelines require it at the name's first appearance in a creative, and this
 * row is a creative of its own. `landing-band.test.ts` compares these display
 * strings against the console's `PLATFORM_NAME` and `PLATFORM_ORDER`, so the
 * row accounts for every platform the release manifest can carry, and the two
 * move together.
 */
export const platforms = [
	{
		key: 'windows',
		name: 'Windows',
		file: null,
		glyph: 'desktop',
		why: 'Microsoft requires a trademark use licence for the Windows symbol, which is not held, and permits the name in text meanwhile.'
	},
	{
		key: 'macos',
		name: 'macOS',
		file: null,
		glyph: 'desktop',
		why: 'Apple’s marketing guidelines forbid the standalone Apple logo, so the row draws a desktop glyph of ours.'
	},
	{
		key: 'linux',
		name: 'Linux',
		file: null,
		glyph: 'desktop',
		why: 'Tux is licensed for attribution-free use but the landing draws only marks with a vendor policy on file.'
	},
	{ key: 'android', name: 'Android™', file: 'android-robot.svg', why: null }
];

/** The sentences the licences require, printed under the row.
 *
 * Google asks that its Creative Commons line appear "in the creative", and this
 * row is the creative; Microsoft's guidelines ask that complete compatibility
 * information sit in plain text beside the symbol; Apple's and the Linux
 * Foundation's ask for a credit line where their marks are named. A caption
 * under the row is what satisfies all of them, where a footer would satisfy
 * none cleanly.
 */
export const attribution = [
	'The Android robot is reproduced or modified from work created and shared by Google and used according to terms described in the Creative Commons 3.0 Attribution License.',
	'Android is a trademark of Google LLC.',
	'Windows is a trademark of the Microsoft group of companies.',
	'macOS is a trademark of Apple Inc., registered in the U.S. and other countries.',
	'Linux® is the registered trademark of Linus Torvalds in the U.S. and other countries.'
];
