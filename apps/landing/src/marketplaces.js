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
 * Three entries carry no file. Etsy and Shopify both state that their logo may
 * not be used without written permission, and this is a public marketing page
 * rather than the page behind a login the founder's 2026-09-05 decision
 * covered; Boom Learning's guidelines make a permission statement mandatory
 * wherever its mark appears, and we do not have permission to make one. Each
 * draws its initial and its name until permission is reported.
 *
 * `live` is the two marketplaces that connect today, which is the claim
 * `site.js`'s availability sentence makes and the only one the caption repeats.
 */
export const marks = [
	{ name: 'TPT', home: 'https://www.teacherspayteachers.com/', file: 'tpt-mark.png', shape: 'icon', live: true },
	{ name: 'TES', home: 'https://www.tes.com/', file: 'tes-mark.png', shape: 'icon', live: true },
	{ name: 'Etsy', home: 'https://www.etsy.com/' },
	{ name: 'Shopify', home: 'https://www.shopify.com/' },
	{ name: 'Made By Teachers', home: 'https://madebyteachers.com/', file: 'made-by-teachers.jpg', shape: 'icon' },
	{ name: 'Classful', home: 'https://classful.com/', file: 'classful.svg', shape: 'wordmark' },
	{ name: 'Boom Learning', home: 'https://www.boomlearning.com/' },
	{ name: 'Teach Simple', home: 'https://teachsimple.com/', file: 'teach-simple.svg', shape: 'wordmark' },
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
