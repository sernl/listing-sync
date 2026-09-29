import { readdirSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { authPageOf, preloadScript } from './shell-preload';

const ROOT = '/home/someone/web';

describe('authPageOf', () => {
	it('names the path each signed-out page answers', () => {
		expect(authPageOf(`${ROOT}/src/routes/(auth)/login/+page.svelte`)).toBe('/login');
		expect(authPageOf(`${ROOT}/src/routes/(auth)/reset/confirm/+page.svelte`)).toBe(
			'/reset/confirm'
		);
		expect(authPageOf(`${ROOT}/src/routes/(auth)/signup/+page.ts`)).toBe('/signup');
	});

	it('leaves out the group layout, which every path shares, and every console page', () => {
		expect(authPageOf(`${ROOT}/src/routes/(auth)/+layout.svelte`)).toBeNull();
		expect(authPageOf(`${ROOT}/src/routes/(console)/resources/+page.svelte`)).toBeNull();
		expect(authPageOf(`${ROOT}/src/routes/(auth)/login/Helper.svelte`)).toBeNull();
		expect(authPageOf(`${ROOT}/src/lib/pages/account/signed-out.css`)).toBeNull();
	});

	it('finds a path for every page the group actually holds', () => {
		const group = new URL('../routes/(auth)/', import.meta.url);
		const pages: string[] = [];
		const walk = (dir: URL) => {
			for (const entry of readdirSync(dir, { withFileTypes: true })) {
				if (entry.isDirectory()) {
					walk(new URL(`${entry.name}/`, dir));
				} else if (entry.name === '+page.svelte') {
					pages.push(new URL(entry.name, dir).pathname);
				}
			}
		};
		walk(group);
		expect(pages.length).toBeGreaterThanOrEqual(4);
		for (const page of pages) {
			expect(authPageOf(page), page).toMatch(/^\/[a-z/]+$/);
		}
	});
});

/** Runs the shell's script against a stub document at `pathname`, and
 *  answers the links it appended. */
function run(files: Record<string, string[]>, pathname: string) {
	const appended: { rel: string; as?: string; crossOrigin?: string; href: string }[] = [];
	const body = preloadScript(files).replace(/^<script>|<\/script>$/g, '');
	const document = {
		createElement: () => ({}) as { rel: string; as?: string; crossOrigin?: string; href: string },
		head: {
			appendChild: (link: { rel: string; as?: string; crossOrigin?: string; href: string }) =>
				appended.push(link)
		}
	};
	new Function('document', 'location', body)(document, { pathname });
	return appended;
}

describe('preloadScript', () => {
	const files = {
		'/login': ['/_app/immutable/assets/AuthFrame.css', '/_app/immutable/nodes/6.js'],
		'/reset/confirm': ['/_app/immutable/nodes/8.js']
	};

	it('preloads the list for the path it was opened at, in the mode the loader asks in, beside the Turnstile connection', () => {
		expect(run(files, '/login')).toEqual([
			{ rel: 'preconnect', href: 'https://challenges.cloudflare.com' },
			{
				rel: 'preload',
				as: 'style',
				crossOrigin: '',
				href: '/_app/immutable/assets/AuthFrame.css'
			},
			{ rel: 'modulepreload', crossOrigin: '', href: '/_app/immutable/nodes/6.js' }
		]);
		expect(run(files, '/reset/confirm/')).toEqual([
			{ rel: 'preconnect', href: 'https://challenges.cloudflare.com' },
			{ rel: 'modulepreload', crossOrigin: '', href: '/_app/immutable/nodes/8.js' }
		]);
	});

	it('preloads nothing and connects nowhere on any other path, a prefix and an inherited name included', () => {
		expect(run(files, '/')).toEqual([]);
		expect(run(files, '/resources')).toEqual([]);
		expect(run(files, '/reset')).toEqual([]);
		expect(run(files, '/login/elsewhere')).toEqual([]);
		expect(run(files, '/constructor')).toEqual([]);
	});
});
