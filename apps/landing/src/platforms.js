/**
 * The platforms this site says a build exists for, and what each may draw.
 *
 * The row exists because the sentence beside it tells a reader that some work
 * needs a small app, and a reader on a phone cannot tell from that sentence
 * whether their phone is one of the two. It is not a download page: `site.js`
 * owns the link, and this names the platforms.
 *
 * A row carries a file only where the platform's owner permits us to draw its
 * mark, so `file` is null far more often than it is missing. Google licenses
 * the Android robot under Creative Commons with an attribution line this row
 * carries; Microsoft requires an express licence for the Windows symbol, which
 * we have not applied for -- the request is drafted in
 * `docs/notes/design/marketplace-logo-sources.md` for the founder to send --
 * and permits the name in text meanwhile. The mark files are copies under
 * `public/vendors/` rather than links to the console's, for the reason
 * `public/marks/` holds copies.
 *
 * The Android name carries its trademark symbol because Google's brand
 * guidelines require it at the name's first appearance in a creative, and this
 * row is a creative of its own. `landing-band.test.ts` compares these display
 * strings against the console's `PLATFORM_NAME`, so the two move together.
 *
 * `omitted` is the other half of the same fact, and it is here rather than
 * unsaid so that a platform can never leave this row silently: the row and the
 * omissions together must account for every platform the release manifest can
 * carry a build for, which `landing-band.test.ts` holds against the console's
 * own `PLATFORM_ORDER`.
 */
export const platforms = [
	{
		name: 'Windows',
		file: null,
		why: 'Microsoft requires a trademark use licence for the Windows symbol, which is not held, and permits the name in text meanwhile.'
	},
	{ name: 'Android™', file: 'android-robot.svg', why: null }
];

/** Platforms the app can publish a build for and this row does not name. */
export const omitted = [{ name: 'macOS', why: 'No macOS build is published.' }];

/** The sentences the two licences require, printed under the row.
 *
 * Google asks that its Creative Commons line appear "in the creative", and this
 * row is the creative; Microsoft's guidelines ask that complete compatibility
 * information sit in plain text beside the symbol. A caption under the row is
 * what satisfies both, where a footer would satisfy neither cleanly.
 */
export const attribution = [
	'The Android robot is reproduced or modified from work created and shared by Google and used according to terms described in the Creative Commons 3.0 Attribution License.',
	'Android is a trademark of Google LLC.',
	'Windows is a trademark of the Microsoft group of companies.'
];
