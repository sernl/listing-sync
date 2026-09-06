/**
 * The marks the hero band draws, transcribed from the console's
 * `web/src/lib/pages/marketplaces/catalogue.ts` in its order.
 *
 * A transcription rather than an import, and the files under `public/marks/`
 * are copies rather than links, for the reason `public/favicon.svg` is a copy
 * of the console's mark: the two trees build separately, so a path served by
 * the console's fallthrough under `tam-server` would 404 under
 * `just landing-dev` and the dev render would disagree with the production
 * one.
 *
 * `marks/` and not `marketplaces/`, which is what these same files are called
 * under `web/static/`. `tam-server` answers a path from this build ahead of
 * the console and matches files rather than directories, so a landing
 * directory sharing a name with a console route is inert only for as long as
 * this build holds no page at that path: a later `marketplaces.astro` would
 * answer `/marketplaces` with a marketing page and take a seller's
 * Marketplaces screen away, with nothing failing to say so. No console route
 * in `web/src/lib/nav.ts` is named `marks`.
 *
 * One entry carries no file. Boom Learning's guidelines make the sentence
 * "Boom™ is the trademark of Boom Learning. Used with permission." mandatory
 * wherever its mark appears, and we hold no permission, so showing the mark
 * means either breaching the rule that governs it or printing something untrue.
 * It draws its initial and its name until permission is reported.
 *
 * Etsy and Shopify carried no file until 2026-09-06, when the founder reversed
 * that half of the decision recorded in `docs/design/decisions.md` and took the
 * same risk here that was taken for the console page: both owners require
 * written permission for logo use, both are being approached, and neither
 * requires a statement we cannot truthfully make. Boom Learning is different in
 * kind rather than a smaller version of the same thing, which is why it stayed.
 *
 * `featured` is the four the founder's mockup draws in the hero row, in the
 * mockup's order; the rest sit behind the "and beyond." disclosure beside them,
 * which is what the Resources link in the header points at.
 *
 * `shape` is what the file is, which decides both the height it is drawn at and
 * whether the row prints the name beside it: an `icon` carries no lettering and
 * takes our own label, a `wordmark` is the name drawn across a wide canvas, and
 * a `square` is a mark drawn inside a square canvas, which needs a square's
 * height before its own lettering can be read at all.
 *
 * The three marks we authored ourselves -- TPT, Tes and Etsy -- are SVG traced
 * from the PNG favicons whose provenance rows are in
 * `docs/notes/design/marketplace-logo-sources.md`, and the files here are byte
 * copies of the console's.
 */
export const marks = [
	{ name: 'TPT', home: 'https://www.teacherspayteachers.com/', file: 'tpt-mark.svg', shape: 'icon', featured: true },
	{ name: 'Tes', home: 'https://www.tes.com/', file: 'tes-mark.svg', shape: 'square', featured: true },
	{ name: 'Classful', home: 'https://classful.com/', file: 'classful.svg', shape: 'wordmark', featured: true },
	{ name: 'Teach Simple', home: 'https://teachsimple.com/', file: 'teach-simple.svg', shape: 'square', featured: true },
	{ name: 'Etsy', home: 'https://www.etsy.com/', file: 'etsy.svg', shape: 'icon' },
	{ name: 'Shopify', home: 'https://www.shopify.com/', file: 'shopify.png', shape: 'icon' },
	{ name: 'Made By Teachers', home: 'https://madebyteachers.com/', file: 'made-by-teachers.jpg', shape: 'icon' },
	{ name: 'Boom Learning', home: 'https://www.boomlearning.com/' },
	{ name: 'Amped Up Learning', home: 'https://ampeduplearning.com/', file: 'amped-up-learning.png', shape: 'wordmark' },
	{ name: 'Teacha!', home: 'https://www.teacharesources.com/', file: 'teacha.svg', shape: 'icon' },
	{ name: 'TeachShare', home: 'https://www.teachshare.com/', file: 'teachshare.svg', shape: 'icon' },
	{ name: 'eduki', home: 'https://eduki.com/', file: 'eduki.png', shape: 'wordmark' },
	{ name: 'TeachBuySell', home: 'https://teachbuysell.com.au/', file: 'teachbuysell.png', shape: 'icon' },
	{ name: 'TeachMzantsi', home: 'https://teachmzantsi.com/', file: 'teach-mzantsi.png', shape: 'wordmark' },
	{ name: 'Lesson Planned', home: 'https://lessonplanned.co.uk/', file: 'lesson-planned.png', shape: 'icon' },
	{ name: 'School Ninja', home: 'https://schoolninja.au/', file: 'school-ninja.png', shape: 'icon' },
	{ name: 'TPD', home: 'https://tpd.edu.au/', file: 'tpd.jpg', shape: 'icon' },
	{ name: 'Gumroad', home: 'https://gumroad.com/', file: 'gumroad.svg', shape: 'icon' },
	{ name: 'Payhip', home: 'https://payhip.com/', file: 'payhip.png', shape: 'icon' },
	{ name: 'Sellfy', home: 'https://sellfy.com/', file: 'sellfy.svg', shape: 'icon' },
	{ name: 'Lemon Squeezy', home: 'https://www.lemonsqueezy.com/', file: 'lemon-squeezy.jpg', shape: 'icon' }
];
