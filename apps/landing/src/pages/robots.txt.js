/* The landing pages are for crawlers; nothing else on this origin is.
   `tam-server` answers a path from the landing build when the build holds
   it and hands every other path to the console (`crates/tam-server/src/
   serving.rs`, `route`), whose pages are drawn in the browser behind a
   sign-in, so each first segment the console routes under is listed here,
   with the API, the identity service, the analytics proxy, the installers
   and the maintenance preview. `/app` is anchored because a bare prefix
   would also refuse `/apple-touch-icon.png`. The console's segments are the
   directories under `web/src/routes`, and `served-artefacts` in `flake.nix`
   fails when one of them has no line here. */
const disallowed = [
	'/v1/',
	'/api/',
	'/ingest/',
	'/downloads/',
	'/maintenance/',
	'/_app/',
	'/app$',
	'/app/',
	'/login',
	'/signup',
	'/reset',
	'/account',
	'/admin',
	'/analytics',
	'/automations',
	'/collections',
	'/connections',
	'/export',
	'/guides',
	'/help',
	'/import',
	'/imports',
	'/inventory',
	'/jobs',
	'/labels',
	'/library',
	'/listings',
	'/marketplaces',
	'/notifications',
	'/purchases',
	'/queue',
	'/reconciliation',
	'/resources',
	'/settings',
	'/status',
	'/sync',
	'/templates'
];

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
