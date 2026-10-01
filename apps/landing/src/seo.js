/**
 * What search engines and link previews read about this site: the pages the
 * sitemap lists, and the schema.org records the layout writes into each
 * page's head. Every figure comes from the server's plan table by way of
 * `plans.generated.js`, so a price change reaches the structured data in the
 * same build that reaches the cards.
 */
import { PACKS, PLANS } from './plans.generated.js';
import { faqs } from './pricing.js';
import { supportEmail } from './site.js';
import sheet from './styles/site.css?raw';

export const siteName = 'Teachouse';

/**
 * The browser chrome's colour: the light page ground, read out of the token
 * block in `site.css` so the palette stays declared in one place. The site
 * is light until a visitor chooses otherwise, so the light value is the one
 * a first visit paints.
 */
export const themeColor = /:root\s*\{[^}]*?--ground:\s*([^;]+);/.exec(sheet)?.[1].trim();
if (!themeColor) throw new Error('site.css declares no --ground in :root');

/**
 * Every indexable page, in the form its canonical link takes, with the date
 * its text last changed in substance. The build has no git history to read
 * (the Nix build copies the tree without `.git`), so the date is written
 * here: move it in the same commit that changes the page.
 */
export const indexedPages = [
	{ path: '/', lastmod: '2026-10-02' },
	{ path: '/pricing/', lastmod: '2026-10-02' },
	{ path: '/terms/', lastmod: '2026-10-02' },
	{ path: '/privacy/', lastmod: '2026-10-02' }
];

/** A cent figure as the decimal string schema.org prices take. */
const price = (cents) => (cents / 100).toFixed(2);

/** The business, with no address: Teachouse never publishes one. */
export const organization = (site) => ({
	'@type': 'Organization',
	'@id': new URL('/#organization', site).href,
	name: siteName,
	url: new URL('/', site).href,
	logo: new URL('/brand/logo.svg', site).href,
	email: supportEmail,
	contactPoint: {
		'@type': 'ContactPoint',
		contactType: 'customer support',
		email: supportEmail,
		availableLanguage: 'en'
	}
});

/** The product and what it costs, in US dollars as the cards show it. */
export const softwareApplication = (site, description) => ({
	'@type': 'SoftwareApplication',
	name: siteName,
	description,
	url: new URL('/', site).href,
	applicationCategory: 'BusinessApplication',
	operatingSystem: 'Web, Windows, macOS, Linux, Android',
	publisher: { '@id': new URL('/#organization', site).href },
	offers: [
		...PLANS.map((plan) => ({
			'@type': 'Offer',
			name: plan.name,
			description: plan.tagline,
			price: price(plan.monthly_cents ?? 0),
			priceCurrency: 'USD',
			url: new URL('/pricing/', site).href,
			...(plan.monthly_cents === null
				? {}
				: {
						priceSpecification: {
							'@type': 'UnitPriceSpecification',
							price: price(plan.monthly_cents),
							priceCurrency: 'USD',
							billingDuration: 'P1M'
						}
					})
		})),
		...PACKS.map((pack) => ({
			'@type': 'Offer',
			name: `${pack.moves} moves (one-off)`,
			price: price(pack.price_cents),
			priceCurrency: 'USD',
			url: new URL('/pricing/', site).href
		}))
	]
});

/** The FAQ section's questions and answers, as the page draws them. */
export const faqPage = () => ({
	'@type': 'FAQPage',
	mainEntity: faqs.map((item) => ({
		'@type': 'Question',
		name: item.q,
		acceptedAnswer: {
			'@type': 'Answer',
			text: item.a
		}
	}))
});

/** Home, then this page. */
export const breadcrumbs = (site, path, name) => ({
	'@type': 'BreadcrumbList',
	itemListElement: [
		{ '@type': 'ListItem', position: 1, name: siteName, item: new URL('/', site).href },
		{ '@type': 'ListItem', position: 2, name, item: new URL(path, site).href }
	]
});

/**
 * One `@graph` document as the text of a `<script type="application/ld+json">`.
 * `<` is escaped so no string in it can close the element early.
 */
export const jsonLd = (nodes) =>
	JSON.stringify({ '@context': 'https://schema.org', '@graph': nodes }).replaceAll('<', '\\u003c');
