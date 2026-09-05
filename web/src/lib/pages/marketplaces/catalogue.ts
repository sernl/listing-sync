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
	| { kind: 'image'; src: string }
	| { kind: 'wordmark'; text: string };

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
		mark: { kind: 'image', src: '/marketplaces/tes-mark.png' },
		home: 'https://www.tes.com/'
	},
	{
		marketplace: 'Tpt',
		name: 'TPT',
		mark: { kind: 'image', src: '/marketplaces/tpt-mark.png' },
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
		mark: { kind: 'image', src: '/marketplaces/etsy.png' },
		home: 'https://www.etsy.com/',
		body: 'Etsy publishes an official seller API, so this one would run on our servers.',
		marketplace: 'Etsy'
	},
	{
		slug: 'shopify',
		name: 'Shopify',
		mark: { kind: 'image', src: '/marketplaces/shopify.svg' },
		home: 'https://www.shopify.com/',
		body: 'Shopify publishes an official Admin API, so this one would run on our servers.'
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
		mark: { kind: 'image', src: '/marketplaces/made-by-teachers.jpg' },
		home: 'https://madebyteachers.com/',
		body: 'No official API is published, so this one would run on your own device.'
	},
	{
		slug: 'classful',
		name: 'Classful',
		mark: { kind: 'image', src: '/marketplaces/classful.svg' },
		home: 'https://classful.com/',
		body: 'No official API is published, so this one would run on your own device.'
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
		body: 'No official API is published, so this one would run on your own device.'
	},
	{
		slug: 'teach-simple',
		name: 'Teach Simple',
		mark: { kind: 'image', src: '/marketplaces/teach-simple.svg' },
		home: 'https://teachsimple.com/',
		body: 'No official API is published, so this one would run on your own device.'
	},
	{
		slug: 'amped-up-learning',
		name: 'Amped Up Learning',
		mark: { kind: 'image', src: '/marketplaces/amped-up-learning.png' },
		home: 'https://ampeduplearning.com/',
		body: 'No official API is published, so this one would run on your own device.'
	},
	{
		slug: 'teacha',
		name: 'Teacha!',
		mark: { kind: 'image', src: '/marketplaces/teacha.png' },
		home: 'https://www.teacharesources.com/',
		body: 'No official API is published, so this one would run on your own device.'
	},
	{
		slug: 'teachshare',
		name: 'TeachShare',
		mark: { kind: 'image', src: '/marketplaces/teachshare.svg' },
		home: 'https://www.teachshare.com/',
		body: 'No official API is published, so this one would run on your own device.'
	},
	{
		slug: 'eduki',
		name: 'eduki',
		mark: { kind: 'image', src: '/marketplaces/eduki.png' },
		home: 'https://eduki.com/',
		body: 'No official API is published, so this one would run on your own device.'
	},
	{
		slug: 'teachbuysell',
		name: 'TeachBuySell',
		mark: { kind: 'image', src: '/marketplaces/teachbuysell.png' },
		home: 'https://teachbuysell.com.au/',
		body: 'No official API is published, so this one would run on your own device.'
	},
	{
		slug: 'teach-mzantsi',
		name: 'TeachMzantsi',
		mark: { kind: 'image', src: '/marketplaces/teach-mzantsi.png' },
		home: 'https://teachmzantsi.com/',
		body: 'No official API is published, so this one would run on your own device.'
	},
	{
		slug: 'lesson-planned',
		name: 'Lesson Planned',
		mark: { kind: 'image', src: '/marketplaces/lesson-planned.png' },
		home: 'https://lessonplanned.co.uk/',
		body: 'No official API is published, so this one would run on your own device.'
	},
	{
		slug: 'school-ninja',
		name: 'School Ninja',
		mark: { kind: 'image', src: '/marketplaces/school-ninja.png' },
		home: 'https://schoolninja.au/',
		body: 'No official API is published, so this one would run on your own device.'
	},
	{
		slug: 'tpd',
		name: 'TPD',
		mark: { kind: 'image', src: '/marketplaces/tpd.png' },
		home: 'https://tpd.edu.au/',
		body: 'No official API is published, so this one would run on your own device.'
	},
	{
		slug: 'gumroad',
		name: 'Gumroad',
		mark: { kind: 'image', src: '/marketplaces/gumroad.svg' },
		home: 'https://gumroad.com/',
		body: 'Gumroad publishes an official API, so this one would run on our servers.'
	},
	{
		slug: 'payhip',
		name: 'Payhip',
		mark: { kind: 'image', src: '/marketplaces/payhip.svg' },
		home: 'https://payhip.com/',
		body: "Payhip's public API does not yet cover products, so we are waiting on it."
	},
	{
		slug: 'sellfy',
		name: 'Sellfy',
		mark: { kind: 'image', src: '/marketplaces/sellfy.svg' },
		home: 'https://sellfy.com/',
		body: 'Sellfy publishes no product API, so this one would run on your own device.'
	},
	{
		slug: 'lemon-squeezy',
		name: 'Lemon Squeezy',
		mark: { kind: 'image', src: '/marketplaces/lemon-squeezy.svg' },
		home: 'https://www.lemonsqueezy.com/',
		body: 'Lemon Squeezy publishes an official API, so this one would run on our servers.'
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

/** The 40px tile, less its 4px of padding on each side. */
const TILE_INNER_PX = 32;

/** How wide a Fraunces glyph runs against its own point size, averaged over the
 *  mixed-case words this tile holds. Close enough to fit a word, and the clamp
 *  below catches where it is not. */
const GLYPH_WIDTH_RATIO = 0.5;

/**
 * What point size sets `text` across a logo tile without clipping it.
 *
 * A wordmark tile is a fixed 40px square holding a word of no fixed length, so
 * the size follows the word. Below 8px the name stops being readable and above
 * 15px it stops looking like a mark, so both ends are clamped. A word long
 * enough to reach the floor is one whose tile text should be shortened instead;
 * the floor keeps it legible rather than letting it shrink to nothing, and the
 * catalogue's own test says none of the words we draw gets near it.
 */
export function wordmarkSize(text: string): number {
	const fitted = TILE_INNER_PX / (text.length * GLYPH_WIDTH_RATIO);
	return Math.max(8, Math.min(15, Math.round(fitted)));
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
