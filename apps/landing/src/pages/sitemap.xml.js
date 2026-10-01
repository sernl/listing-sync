import { indexedPages } from '../seo.js';

/* The four public pages and when each last changed, built with the site so a
   page added to `indexedPages` is listed by the same build. Served at the
   root by `tam-server` like any other file of the build. */
export function GET({ site }) {
	const urls = indexedPages
		.map(
			(page) =>
				`<url><loc>${new URL(page.path, site).href}</loc><lastmod>${page.lastmod}</lastmod></url>`
		)
		.join('');
	return new Response(
		`<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">${urls}</urlset>\n`,
		{ headers: { 'Content-Type': 'application/xml; charset=utf-8' } }
	);
}
