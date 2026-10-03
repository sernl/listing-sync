import { describe, expect, it } from 'vitest';
import { type ModuleGraph, planChunks } from './chunk-groups';

const W = '/home/someone/web';
const node = (n: number) => `${W}/.svelte-kit/generated/client-optimized/nodes/${n}.js`;
const lib = (name: string) => `${W}/src/lib/${name}`;
const route = (path: string) => `${W}/src/routes/${path}`;
const START = `${W}/node_modules/@sveltejs/kit/src/runtime/client/entry.js`;
const CLIENT = `${W}/node_modules/@sveltejs/kit/src/runtime/client/client.js`;
const APP = `${W}/.svelte-kit/generated/client-optimized/app.js`;
const QUERY = `${W}/node_modules/@tanstack/query-core/build/modern/index.js`;
const PDFJS = `${W}/node_modules/pdfjs-dist/build/pdf.mjs`;

// A small console: the shell, the signed-out group with two screens, the
// console frame and one console page. pdf.js is only ever `import()`ed, so it
// is in the graph but no static import reaches it.
const imports: Record<string, readonly string[]> = {
	[START]: [CLIENT],
	[APP]: [`${W}/src/hooks.client.ts`],
	[`${W}/src/hooks.client.ts`]: [lib('lost-chunk.ts')],
	[node(0)]: [route('+layout.ts')],
	[route('+layout.ts')]: [lib('site.ts')],
	[node(1)]: [route('+error.svelte')],
	[node(2)]: [route('(auth)/+layout.svelte')],
	[route('(auth)/+layout.svelte')]: [lib('AuthFrame.svelte'), lib('Button.svelte')],
	[node(3)]: [route('(console)/+layout.svelte')],
	[route('(console)/+layout.svelte')]: [lib('Console.svelte')],
	[lib('Console.svelte')]: [lib('Button.svelte'), lib('api.ts'), QUERY],
	[lib('api.ts')]: [lib('http.ts')],
	[node(5)]: [route('(auth)/login/+page.svelte')],
	[route('(auth)/login/+page.svelte')]: [lib('captcha.ts'), lib('Field.svelte')],
	[node(6)]: [route('(auth)/signup/+page.svelte')],
	[route('(auth)/signup/+page.svelte')]: [lib('captcha.ts'), lib('api.ts')],
	[node(7)]: [route('(console)/resources/+page.svelte')],
	[route('(console)/resources/+page.svelte')]: [
		lib('pages/resources/list.ts'),
		lib('Field.svelte'),
		lib('pages/resources/PreviewMaker.svelte')
	]
};

const graph: ModuleGraph = {
	ids: [...new Set([...Object.keys(imports), ...Object.values(imports).flat(), PDFJS])],
	staticImports: (id) => imports[id] ?? [],
	isEntry: (id) => id === START || id === APP || id.includes('/nodes/')
};

describe('planChunks', () => {
	const plan = planChunks(graph);

	it('writes everything every path runs into one shell chunk', () => {
		for (const id of [CLIENT, `${W}/src/hooks.client.ts`, lib('lost-chunk.ts'), lib('site.ts')]) {
			expect(plan.get(id), id).toBe('shell');
		}
		expect(plan.get(route('+error.svelte'))).toBe('shell');
	});

	it('writes what every signed-in page runs into one console chunk', () => {
		expect(plan.get(route('(console)/+layout.svelte'))).toBe('console');
		expect(plan.get(lib('Console.svelte'))).toBe('console');
		expect(plan.get(QUERY)).toBe('console');
	});

	it('keeps the console out of a signed-out screen by sharing only what both reach, per screen', () => {
		expect(plan.get(lib('Button.svelte'))).toBe('shared:layout');
		expect(plan.get(lib('Field.svelte'))).toBe('shared:login');
		// The sign-up screen needs the API client and the sign-in screen does
		// not, so it is not in anything the sign-in screen loads.
		expect(plan.get(lib('api.ts'))).toBe('shared:signup');
		expect(plan.get(lib('http.ts'))).toBe('shared:signup');
	});

	it('leaves a page’s own modules, signed-out-only modules and lazy ones to the bundler', () => {
		for (const id of [
			lib('pages/resources/list.ts'),
			route('(console)/resources/+page.svelte'),
			lib('captcha.ts'),
			route('(auth)/login/+page.svelte'),
			PDFJS
		]) {
			expect(plan.get(id), id).toBeUndefined();
		}
	});

	it('never groups an entry, which would turn it into a facade', () => {
		for (const id of [START, APP, node(0), node(3), node(7)]) {
			expect(plan.get(id), id).toBeUndefined();
		}
	});

	it('names a chunk for every static import of a grouped module', () => {
		// The bundler pulls an unnamed import of a grouped module into that
		// group; that is how the console's code would end up in the sign-in
		// screen's chunks.
		for (const [id, group] of plan) {
			for (const imported of graph.staticImports(id)) {
				expect(plan.get(imported), `${id} (${group}) imports ${imported}`).toBeDefined();
			}
		}
	});
});
