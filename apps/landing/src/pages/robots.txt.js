/* This file is served on the marketing host only. The console moved to its
   own host (`consoleUrl` in `src/site.js`), which answers its own robots.txt,
   so no console route is listed here. What this host still answers and no
   crawler wants is: the API it keeps for the site's banner and plans, the
   identity service and the analytics proxy it redirects to the console host,
   the installers, and the maintenance preview. `served-artefacts` in
   `flake.nix` fails when one of the first four is missing or when a console
   route segment comes back. */
const disallowed = ['/v1/', '/api/', '/ingest/', '/downloads/', '/maintenance/'];

export function GET({ site }) {
	const lines = [
		'User-agent: *',
		'Allow: /',
		...disallowed.map((path) => `Disallow: ${path}`),
		'',
		`Sitemap: ${new URL('/sitemap.xml', site).href}`,
		''
	];
	return new Response(lines.join('\n'), {
		headers: { 'Content-Type': 'text/plain; charset=utf-8' }
	});
}
