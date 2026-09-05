// Which marketplaces the Marketplaces page tiles, in what order, and what each
// tile says.
//
// Every sentence here is copy that is true on the day it was written: two
// marketplaces are connected, two more publish an official API and are next,
// and the rest are places sellers told us they sell. Nothing on this page
// promises a date, and nothing describes a relationship we do not have.
//
// The source is the pair of marketplace catalogues researched on 2026-09-05,
// `docs/notes/design/marketplace-catalogue-1.md` and `-2.md`; every claim in
// them carries the URL it was read from, this file's homepage addresses
// included. Order is catalogue one's own recommended
// ordering, with catalogue
// two's teaching marketplaces appended to the teaching group in the order it
// introduces them. Everything either catalogue told us not to tile is absent --
// closed, invitation-only, course platforms and subscription publishers -- and
// so is everything it held in reserve, except the three the founder asked for
// by name on 2026-09-05: TeachBuySell, School Ninja and TPD, the last two of
// which catalogue two upgraded from unverified to verified live marketplaces.

import type { Marketplace } from '$lib/generated/vocab';

/** How a tile draws its mark.
 *
 *  A `wordmark` is not a fallback for a missing file. Every marketplace whose
 *  logo we could source shows it, under the disclaimer beneath the grid, which
 *  is the founder's decision of 2026-09-05. Boom Learning is the single
 *  exception, and the two browsers are not marketplaces and publish nothing we
 *  fetched. `docs/notes/design/marketplace-logo-sources.md` records every file,
 *  where it came from, and why the one exception is one. */
export type Mark =
	| { kind: 'image'; src: string; shape: Shape }
	| { kind: 'wordmark'; text: string };

/** Which tile a mark is drawn in, which follows the mark's own proportions.
 *
 *  An `icon` is square or near it, a glyph drawn to sit in a square, and takes
 *  the square tile. A `wordmark` is wider than it is tall and takes the wide
 *  one, because a square tile draws a five-to-one wordmark nine pixels tall.
 *  A marketplace's own app icon is the usual source of a square mark, but a
 *  square logo we already held is one too, which is why TES and TPT are icons
 *  without an icon ever having been fetched for them.
 *
 *  It sits on the mark rather than on the tile because the card is handed the
 *  mark, not the tile, and the page that hands it over is not this pass's to
 *  edit. A text mark carries no shape: it has no file and is never square. */
export type Shape = 'icon' | 'wordmark';

/** One marketplace we do not work with yet.
 *
 *  `marketplace` is present only where the marketplace is a member of the Rust
 *  `Marketplace` enum, which today is Etsy alone among these: a transport class
 *  is a recorded fact about a marketplace the platform knows, and a tile for
 *  one it does not know has no class to badge. The expected branch of those
 *  reaches the seller through `body` instead, in the catalogue's own words. */
export interface ProspectTile {
	slug: string;
	name: string;
	mark: Mark;
	/** One sentence, from the catalogue's tile-ordering copy. */
	body: string;
	/** The marketplace's own front page, as the catalogue recorded it. Absent
	 *  only on the two browser tiles, which are not marketplaces. */
	home?: string;
	marketplace?: Marketplace;
}

/** One marketplace the platform actually works with.
 *
 *  It carries no body of its own: what a connected marketplace says is read
 *  from the connection and device registry at render time, so a card cannot
 *  state something the data does not. */
export interface LiveTile {
	marketplace: Marketplace;
	/** Capitals, which is how both live marketplaces write their own names. */
	name: string;
	mark: Mark;
	/** The marketplace's own front page. Neither of these two was contacted for
	 *  the research, so both addresses are the ones this repository already
	 *  holds rather than ones read off their sites. */
	home: string;
}

export const LIVE: readonly LiveTile[] = [
	{
		marketplace: 'Tes',
		name: 'TES',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/tes-mark.png' },
		home: 'https://www.tes.com/'
	},
	{
		marketplace: 'Tpt',
		name: 'TPT',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/tpt-mark.png' },
		home: 'https://www.teacherspayteachers.com/'
	}
];

/** The two marketplaces we plan next.
 *
 *  Both publish an official seller API and issue a token for the purpose, which
 *  is what puts them on the server branch of D1 and what separates them from
 *  everything in `LISTED`. Both also state that their logo may not be used
 *  without written permission; the founder decided on 2026-09-05 to show every
 *  marketplace's mark under the disclaimer beneath the grid and to approach
 *  each marketplace, and `docs/notes/design/marketplace-logo-sources.md`
 *  records that. */
export const PLANNED: readonly ProspectTile[] = [
	{
		slug: 'etsy',
		name: 'Etsy',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/etsy.png' },
		home: 'https://www.etsy.com/',
		body: 'Etsy has an official API, so this one would run on our servers.',
		marketplace: 'Etsy'
	},
	{
		slug: 'shopify',
		name: 'Shopify',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/shopify.png' },
		home: 'https://www.shopify.com/',
		body: 'Shopify has an official API, so this one would run on our servers.'
	}
];

/** Where else teachers sell, and what building each would take.
 *
 *  The second clause of every sentence is the transport class the catalogue
 *  derived from whether the marketplace publishes an official API. It is the
 *  honest thing to say about a marketplace nobody has built: the branch follows
 *  from a fact about them, not from a decision of ours. */
export const LISTED: readonly ProspectTile[] = [
	{
		slug: 'made-by-teachers',
		name: 'Made By Teachers',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/made-by-teachers.jpg' },
		home: 'https://madebyteachers.com/',
		body: 'This one has no official API, so it would run on your own computer.'
	},
	{
		slug: 'classful',
		name: 'Classful',
		mark: { kind: 'image', shape: 'wordmark', src: '/marketplaces/classful.svg' },
		home: 'https://classful.com/',
		body: 'This one has no official API, so it would run on your own computer.'
	},
	{
		// The one mark shown as a wordmark. Boom Learning's guidelines make a
		// disclaimer mandatory wherever the logo appears -- "Boom(tm) is the
		// trademark of Boom Learning. Used with permission." -- and we do not
		// have permission, so printing it would be a false statement rather
		// than the accepted guideline risk the founder took for the rest.
		slug: 'boom-learning',
		name: 'Boom Learning',
		mark: { kind: 'wordmark', text: 'Boom' },
		home: 'https://www.boomlearning.com/',
		body: 'This one has no official API, so it would run on your own computer.'
	},
	{
		slug: 'teach-simple',
		name: 'Teach Simple',
		// Wide, though 421 by 362 is nearly square, because the file is a lockup
		// rather than a glyph: an owl above a separate `TeachSimple` wordmark. In
		// the square tile the owl stays legible and the word falls to about five
		// pixels, which is below the nine that had `eduki-icon.png` and
		// `teach-mzantsi-icon.webp` rejected. Its ratio of 1.16 is also a 16 per
		// cent deviation, outside the ten per cent the provenance note holds every
		// other icon on this page to.
		mark: { kind: 'image', shape: 'wordmark', src: '/marketplaces/teach-simple.svg' },
		home: 'https://teachsimple.com/',
		body: 'This one has no official API, so it would run on your own computer.'
	},
	{
		slug: 'amped-up-learning',
		name: 'Amped Up Learning',
		mark: { kind: 'image', shape: 'wordmark', src: '/marketplaces/amped-up-learning.png' },
		home: 'https://ampeduplearning.com/',
		body: 'This one has no official API, so it would run on your own computer.'
	},
	{
		slug: 'teacha',
		name: 'Teacha!',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/teacha.svg' },
		home: 'https://www.teacharesources.com/',
		body: 'This one has no official API, so it would run on your own computer.'
	},
	{
		slug: 'teachshare',
		name: 'TeachShare',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/teachshare.svg' },
		home: 'https://www.teachshare.com/',
		body: 'This one has no official API, so it would run on your own computer.'
	},
	{
		slug: 'eduki',
		name: 'eduki',
		// Not an `icon`, though an `eduki-icon.png` was fetched, because that file
		// is this same wordmark centred on a square canvas: its ink measures 131
		// by 46, so a square tile would draw the word nine pixels tall. eduki
		// publishes no square glyph, so the wordmark in the wide tile is the
		// largest this mark can be drawn.
		//
		// The file itself was cropped for the same reason. eduki published a 643
		// by 208 wordmark inside a 1018 by 880 white canvas; the bytes here are
		// that canvas cropped to the mark and nothing else, and the provenance
		// note records the crop.
		mark: { kind: 'image', shape: 'wordmark', src: '/marketplaces/eduki.png' },
		home: 'https://eduki.com/',
		body: 'This one has no official API, so it would run on your own computer.'
	},
	{
		slug: 'teachbuysell',
		name: 'TeachBuySell',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/teachbuysell.png' },
		home: 'https://teachbuysell.com.au/',
		body: 'This one has no official API, so it would run on your own computer.'
	},
	{
		slug: 'teach-mzantsi',
		name: 'TeachMzantsi',
		// A `teach-mzantsi-icon.webp` was fetched and is deliberately not used: it
		// is a cropped promotional image rather than a glyph, carrying an address
		// line that cannot be read at tile size. A mark that says something
		// illegible says less than the wordmark does.
		mark: { kind: 'image', shape: 'wordmark', src: '/marketplaces/teach-mzantsi.png' },
		home: 'https://teachmzantsi.com/',
		body: 'This one has no official API, so it would run on your own computer.'
	},
	{
		slug: 'lesson-planned',
		name: 'Lesson Planned',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/lesson-planned.png' },
		home: 'https://lessonplanned.co.uk/',
		body: 'This one has no official API, so it would run on your own computer.'
	},
	{
		slug: 'school-ninja',
		name: 'School Ninja',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/school-ninja.png' },
		home: 'https://schoolninja.au/',
		body: 'This one has no official API, so it would run on your own computer.'
	},
	{
		slug: 'tpd',
		name: 'TPD',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/tpd.jpg' },
		home: 'https://tpd.edu.au/',
		body: 'This one has no official API, so it would run on your own computer.'
	},
	{
		slug: 'gumroad',
		name: 'Gumroad',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/gumroad.svg' },
		home: 'https://gumroad.com/',
		body: 'Gumroad has an official API, so this one would run on our servers.'
	},
	{
		slug: 'payhip',
		name: 'Payhip',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/payhip.png' },
		home: 'https://payhip.com/',
		body: "Payhip's API does not cover products yet, so we are waiting on it."
	},
	{
		slug: 'sellfy',
		name: 'Sellfy',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/sellfy.svg' },
		home: 'https://sellfy.com/',
		body: 'Sellfy has no product API, so this one would run on your own computer.'
	},
	{
		slug: 'lemon-squeezy',
		name: 'Lemon Squeezy',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/lemon-squeezy.jpg' },
		home: 'https://www.lemonsqueezy.com/',
		body: 'Lemon Squeezy has an official API, so this one would run on our servers.'
	}
];

/** The two browsers an extension would ship for.
 *
 *  Neither vendor's mark is fetched: no catalogue recorded one, and neither is
 *  a marketplace, so the tile is the browser's name in our own typeface. */
export const EXTENSIONS: readonly ProspectTile[] = [
	{
		slug: 'chrome',
		name: 'Chrome',
		mark: { kind: 'wordmark', text: 'Chrome' },
		body: 'Not available yet.'
	},
	{
		slug: 'firefox',
		name: 'Firefox',
		mark: { kind: 'wordmark', text: 'Firefox' },
		body: 'Not available yet.'
	}
];

/** How much of the mark tile a word may cross: the 96px tile, less its 1px
 *  border and its 8px of padding on each side. */
const TILE_INNER_PX = 78;

/** How wide a Fraunces glyph runs against its own point size: an upper bound
 *  over the words this tile holds, not an average over them.
 *
 *  An average is what the tile had, and it clipped. Measured at 22px in the
 *  rendered page, `Boom` runs 0.700 of its point size per character, `Chrome`
 *  0.636 and `Firefox` 0.483 — a spread wide enough that the mean sized
 *  `Chrome` to 84px across a 78px tile, and `overflow: hidden` took the rest
 *  silently. Only a bound at the widest word makes the function's own promise
 *  true, so 0.700 it is: a word of narrow letters is then set smaller than it
 *  strictly needs, which costs a few points of size and cannot cut a mark in
 *  half. */
const GLYPH_WIDTH_RATIO = 0.7;

/**
 * What point size sets `text` across a logo tile without clipping it.
 *
 * A wordmark tile is a fixed 96 by 56 holding a word of no fixed length, so the
 * size follows the word. Below 8px the name stops being readable and above 22px
 * it stops looking like a mark and starts competing with the 17px name beside
 * it, so both ends are clamped. A word long enough to reach the floor is one
 * whose tile text should be shortened instead; the floor keeps it legible
 * rather than letting it shrink to nothing, and the catalogue's own test says
 * none of the words we draw gets near it.
 *
 * The ceiling binds only at five characters or fewer, since 78 / (5 * 0.7) is
 * 22.3 and a sixth character puts the fitted size under the cap. Of the three
 * words this page draws, that is Boom alone: Boom sets at 22, Chrome at 19 and
 * Firefox at 16. The three wordmark tiles therefore read at three sizes, not
 * one, which is the price of a bound that no word can overflow rather than an
 * average that Chrome overflowed by six pixels.
 */
export function wordmarkSize(text: string): number {
	const fitted = TILE_INNER_PX / (text.length * GLYPH_WIDTH_RATIO);
	return Math.max(8, Math.min(22, Math.round(fitted)));
}

/** What sits under the grid, and under nothing else.
 *
 *  Verbatim from the catalogue, which drafted it as the two sentences that
 *  claim no relationship we do not have. One line under the whole grid rather
 *  than one per tile, which is the catalogue's own recommendation. */
export const DISCLAIMER =
	'The marketplace names and logos shown on this page belong to their respective owners. ' +
	'Teachouse is not affiliated with or endorsed by them, and shows their names and logos ' +
	'only to identify which marketplace each entry refers to.';
