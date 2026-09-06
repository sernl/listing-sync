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
import type { IconName } from '$lib/icons';

/** How a tile draws its mark.
 *
 *  A `wordmark` is not a fallback for a missing file. Every marketplace whose
 *  logo we could source shows it, under the disclaimer beneath the grid, which
 *  is the founder's decision of 2026-09-05. Boom Learning is the single
 *  exception among the marketplaces.
 *  `docs/notes/design/marketplace-logo-sources.md` records every file, where it
 *  came from, and why the one exception is one.
 *
 *  A `glyph` is a mark of our own for the two download tiles whose owner permits
 *  us nothing we can use: Microsoft licenses its symbol and Apple forbids its
 *  logo outright, and all three store badges are licensed only to link to a
 *  store listing we do not have. It is a neutral drawing rather than a logo, so
 *  it claims nothing and needs no provenance row; substituting a real mark the
 *  day permission arrives is a change of this one field. */
export type Mark =
	| { kind: 'image'; src: string; shape: Shape }
	| { kind: 'wordmark'; text: string }
	| { kind: 'glyph'; name: IconName };

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

/** One tile for something we do not work with yet: a marketplace, or a browser
 *  an extension would ship for.
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
	/** The marketplace's own front page, as the catalogue recorded it. On the
	 *  Firefox tile it is Mozilla's own page instead, because Mozilla's policy
	 *  permits its logo in a visual that refers to or links to the program and the
	 *  link is that condition met. Absent on the Chrome tile, which draws no
	 *  vendor mark and so carries no such condition. */
	home?: string;
	marketplace?: Marketplace;
}

/** A prospect tile for a marketplace, which carries a description of it.
 *
 *  The description is required here and absent from `ProspectTile` itself, so
 *  the type says what the founder asked for on 2026-09-06: every marketplace on
 *  this page says what it is, and the two browser tiles are not marketplaces and
 *  are not held to it. Required rather than optional, because an optional
 *  description is one that twenty-one authors each decide about separately, and
 *  a test asserting presence says nothing against a field that may be absent. */
export interface MarketplaceTile extends ProspectTile {
	/** @see LiveTile.about */
	about: string;
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
	/** What this marketplace is, for a seller who does not recognise the name.
	 *
	 *  It sits here rather than reaching the card through `body` because what TES
	 *  is is a fixed fact about TES, where `body` is a fact about the connection
	 *  and is read at render time. So this does not reopen the reason `body` is
	 *  absent from a live tile; it is a different sentence. */
	about: string;
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
		about:
			'A British education company, best known for its teaching magazine, whose marketplace ' +
			'is where many UK teachers buy and sell lesson resources.',
		home: 'https://www.tes.com/'
	},
	{
		marketplace: 'Tpt',
		name: 'TPT',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/tpt-mark.png' },
		about:
			'A large American marketplace for teacher-made classroom resources, where most of the ' +
			'buyers are teachers in the United States.',
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
export const PLANNED: readonly MarketplaceTile[] = [
	{
		slug: 'etsy',
		name: 'Etsy',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/etsy.png' },
		home: 'https://www.etsy.com/',
		body: 'Etsy has an official API, so this one would run on our servers.',
		about:
			'A general marketplace for handmade, vintage and digital goods, where digital downloads ' +
			'including teaching resources sell alongside everything else.',
		marketplace: 'Etsy'
	},
	{
		slug: 'shopify',
		name: 'Shopify',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/shopify.png' },
		home: 'https://www.shopify.com/',
		body: 'Shopify has an official API, so this one would run on our servers.',
		about:
			'Not a marketplace but a shop of your own: Shopify hosts the storefront and takes the ' +
			'payments, and there is no shared catalogue for buyers to find you in.'
	}
];

/** Where else teachers sell, and what building each would take.
 *
 *  The second clause of every sentence is the transport class the catalogue
 *  derived from whether the marketplace publishes an official API. It is the
 *  honest thing to say about a marketplace nobody has built: the branch follows
 *  from a fact about them, not from a decision of ours. */
export const LISTED: readonly MarketplaceTile[] = [
	{
		slug: 'made-by-teachers',
		name: 'Made By Teachers',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/made-by-teachers.jpg' },
		home: 'https://madebyteachers.com/',
		body: 'This one has no official API, so it would run on your own computer.',
		about:
			'A marketplace for teaching resources, which says its listings are all digital ' +
			'downloads from independent teacher-sellers and that those sellers keep 80 per cent ' +
			'of each sale.'
	},
	{
		slug: 'classful',
		name: 'Classful',
		mark: { kind: 'image', shape: 'wordmark', src: '/marketplaces/classful.svg' },
		home: 'https://classful.com/',
		body: 'This one has no official API, so it would run on your own computer.',
		about:
			'An American marketplace for digital classroom resources, which collects and remits US ' +
			"sales tax on the seller's behalf."
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
		body: 'This one has no official API, so it would run on your own computer.',
		about:
			"Sells interactive decks authored in Boom's own studio rather than uploaded files, so " +
			"what you sell there is a deck rather than a file."
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
		body: 'This one has no official API, so it would run on your own computer.',
		about:
			'An American marketplace whose buyers pay a membership rather than a price per item, ' +
			'and which states that half of all revenue goes to the teachers who made the materials.'
	},
	{
		slug: 'amped-up-learning',
		name: 'Amped Up Learning',
		mark: { kind: 'image', shape: 'wordmark', src: '/marketplaces/amped-up-learning.png' },
		home: 'https://ampeduplearning.com/',
		body: 'This one has no official API, so it would run on your own computer.',
		about:
			'Sells teacher-made digital resources alongside apparel, through curated contributor ' +
			'stores rather than one open catalogue.'
	},
	{
		slug: 'teacha',
		name: 'Teacha!',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/teacha.svg' },
		home: 'https://www.teacharesources.com/',
		body: 'This one has no official API, so it would run on your own computer.',
		about:
			'A South African marketplace for teaching resources, which says its sellers keep 65 per ' +
			'cent of each sale.'
	},
	{
		slug: 'teachshare',
		name: 'TeachShare',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/teachshare.svg' },
		home: 'https://www.teachshare.com/',
		body: 'This one has no official API, so it would run on your own computer.',
		about:
			'An American marketplace, run from San Francisco, carrying listings from many sellers ' +
			'at prices from nothing to a few dollars.'
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
		body: 'This one has no official API, so it would run on your own computer.',
		about:
			'A European marketplace for teaching material, started in Germany and priced in euros, ' +
			'whose German-language site is labelled for Germany, Austria and Switzerland.'
	},
	{
		slug: 'teachbuysell',
		name: 'TeachBuySell',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/teachbuysell.png' },
		home: 'https://teachbuysell.com.au/',
		body: 'This one has no official API, so it would run on your own computer.',
		about:
			'An Australian marketplace for primary and early-childhood resources, priced in ' +
			'Australian dollars.'
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
		body: 'This one has no official API, so it would run on your own computer.',
		about:
			'A South African marketplace for teaching and learning resources, priced in rand, where ' +
			'the seller keeps 65 per cent of each sale.'
	},
	{
		slug: 'lesson-planned',
		name: 'Lesson Planned',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/lesson-planned.png' },
		home: 'https://lessonplanned.co.uk/',
		body: 'This one has no official API, so it would run on your own computer.',
		about:
			'A British marketplace where teachers buy and sell original learning resources, priced ' +
			'in pounds and organised around the National Curriculum.'
	},
	{
		slug: 'school-ninja',
		name: 'School Ninja',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/school-ninja.png' },
		home: 'https://schoolninja.au/',
		body: 'This one has no official API, so it would run on your own computer.',
		about:
			'An Australian marketplace where the seller keeps 70 per cent of each sale and must ' +
			'hold an Australian business number to sell at all.'
	},
	{
		slug: 'tpd',
		name: 'TPD',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/tpd.jpg' },
		home: 'https://tpd.edu.au/',
		body: 'This one has no official API, so it would run on your own computer.',
		about:
			'An Australian site trading as Teacher Professional Development, whose payout rises ' +
			'from 60 to 75 per cent as the seller pays for a higher tier.'
	},
	{
		slug: 'gumroad',
		name: 'Gumroad',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/gumroad.svg' },
		home: 'https://gumroad.com/',
		body: 'Gumroad has an official API, so this one would run on our servers.',
		about:
			'A storefront for creators of every kind, selling digital files direct to your own ' +
			'audience, with a discovery feed rather than a marketplace for teaching resources.'
	},
	{
		slug: 'payhip',
		name: 'Payhip',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/payhip.png' },
		home: 'https://payhip.com/',
		body: "Payhip's API does not cover products yet, so we are waiting on it.",
		about:
			'A British platform for selling digital downloads, courses and memberships, running ' +
			'both a shop of your own and a marketplace of its own.'
	},
	{
		slug: 'sellfy',
		name: 'Sellfy',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/sellfy.svg' },
		home: 'https://sellfy.com/',
		body: 'Sellfy has no product API, so this one would run on your own computer.',
		about:
			'A general storefront for selling digital products from a shop page of your own rather ' +
			'than as a marketplace listing.'
	},
	{
		slug: 'lemon-squeezy',
		name: 'Lemon Squeezy',
		mark: { kind: 'image', shape: 'icon', src: '/marketplaces/lemon-squeezy.jpg' },
		home: 'https://www.lemonsqueezy.com/',
		body: 'Lemon Squeezy has an official API, so this one would run on our servers.',
		about:
			'A general store for digital products, owned by Stripe since 2024, which handles the ' +
			'payment and the sales tax on your behalf.'
	}
];

/** The two browsers an extension would ship for.
 *
 *  The two vendors do not permit alike, so the two tiles do not draw alike.
 *  Mozilla's trademark policy, at
 *  https://www.mozilla.org/en-US/foundation/trademarks/policy/, permits without
 *  prior permission the use of "Mozilla logos in visuals to truthfully refer to
 *  and/or to link to the applicable programs", asks for a visible attribution
 *  notice, and forbids modifying or abbreviating the mark. So Firefox draws
 *  Mozilla's own published file unaltered, and `home` is the "link to" half of
 *  that permission rather than decoration.
 *
 *  The same policy settles the heading. It asks that a wordmark be used "only
 *  as an adjective, never as a noun or verb" and that "the generic term for the
 *  Mozilla product or service" follow it, so the tile is headed "Firefox
 *  browser" rather than "Firefox". The file is Mozilla's horizontal
 *  logo-and-wordmark lockup, which already draws the word, and a bare "Firefox"
 *  beside it repeated the wordmark as the noun the policy names.
 *
 *  Google publishes no such allowance for the Chrome icon. Its guidance routes
 *  every product icon through a Partner Marketing Hub approval we have not
 *  applied for, so this tile draws the name, and Google's Chrome branding page
 *  settles how the name is written: "make reference to that Google product by
 *  using the text 'for', 'for use with', or 'compatible with', and be sure to
 *  include the ™ symbol with the Google trademark. Example: 'for Google
 *  Chrome™'". That sentence governs the reference rather than the mark, so the
 *  heading takes Google's full product name and the mark box keeps the short
 *  form: the mark box is `aria-hidden` artwork standing in for a logo we may
 *  not draw, and it is sized by `wordmarkSize`, which sets fourteen characters
 *  below the size this page calls readable. The sentence that goes with it is
 *  in `MARK_ATTRIBUTION`.
 *
 *  A glyph is still not the alternative for Chrome. Lucide carries no brand
 *  marks, so the nearest it offers is a generic browser drawing, and a circular
 *  one would resemble Chrome's own logo more closely than the word does -- which
 *  would make a mark adopted to avoid using a logo the closer imitation of it.
 *  `docs/notes/design/marketplace-logo-sources.md` carries both vendors' terms,
 *  the addresses they were read at and the digest of every file landed. */
export const EXTENSIONS: readonly ProspectTile[] = [
	{
		slug: 'chrome',
		name: 'Google Chrome™',
		mark: { kind: 'wordmark', text: 'Chrome™' },
		body: 'Not available yet.'
	},
	{
		slug: 'firefox',
		name: 'Firefox browser',
		mark: { kind: 'image', shape: 'wordmark', src: '/vendors/firefox.svg' },
		home: 'https://www.mozilla.org/firefox/',
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
 *  0.636 and `Firefox` 0.483, measured while Firefox still drew a wordmark — a
 *  spread wide enough that the mean sized
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
 * 22.3 and a sixth character puts the fitted size under the cap. Of the two
 * words this page draws, that is Boom alone: Boom sets at 22 and Chrome™, whose
 * trademark symbol is a seventh character, at 16. The two wordmark tiles
 * therefore read at two sizes, not one, which is the price of a bound that no
 * word can overflow rather than an average that Chrome overflowed by six
 * pixels.
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
	'The marketplace, browser and platform names and logos shown on this page belong to their ' +
	'respective owners. Teachouse is not affiliated with or endorsed by them, and shows their ' +
	'names and logos only to identify which product each entry refers to.';

/** The sentences the licences this page relies on require, printed under the
 *  grid beside the disclaimer.
 *
 *  An array rather than a paragraph, so each is a row a test can hold against
 *  the mark that obliges it. The first is the grant the Android robot is drawn
 *  under and the third is the notice Mozilla's policy attaches to its logo, so a
 *  sentence dropped in an edit is a licence condition dropped. The other two are
 *  for names set in our own type, which Google requires for its trademarks and
 *  which costs one sentence for Microsoft's.
 *
 *  Verbatim, because a paraphrase of a licence condition is not the condition.
 *  Google asks that its line appear "in the creative", and the page drawing the
 *  robot is the creative, so it sits here rather than in a document a reader
 *  would have to find. `docs/notes/design/marketplace-logo-sources.md` carries
 *  each sentence's source address and the date it was read. */
export const MARK_ATTRIBUTION: readonly string[] = [
	'The Android robot is reproduced or modified from work created and shared by Google and used ' +
		'according to terms described in the Creative Commons 3.0 Attribution License.',
	'Android is a trademark of Google LLC.',
	'Firefox is a trademark of the Mozilla Foundation in the US and other countries.',
	'Google Chrome is a trademark of Google LLC.',
	'Windows is a trademark of the Microsoft group of companies.'
];
