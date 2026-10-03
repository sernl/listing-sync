// Which client chunk a module is written into.
//
// Left to itself the bundler gives every set of route nodes that shares a
// module a chunk of its own. The app shell and the console frame are each
// reached by a different mix of nodes module by module, so a cold load of
// `/resources` asked for 82 scripts, most under 2 kB, and the sign-in screen
// for 39. Each is a request a cold Cloudflare edge can stall on, and one
// stalled script holds the whole route. The modules that are always loaded
// together are written together instead, which removes those requests
// without adding a byte to any page:
//
// - `shell`: everything SvelteKit's two entries, the root layout and the
//   root error page reach. Every path runs it.
// - `shared:<screens>`: what a signed-out screen and a console page both
//   reach beyond the shell, keyed by the signed-out screens that reach it
//   (`layout` when the `(auth)` layout does, which is all of them). The
//   sign-in screen loads its own share and not the sign-up screen's.
// - `console`: what the `(console)` layout and error page reach beyond those.
//   Every signed-in page runs it.
//
// Each layer is closed under static imports given the ones before it, and a
// shared module's screens are a subset of its imports' screens, so a chunk
// only ever imports one from an earlier layer or one keyed by a strictly
// larger set: the chunk graph has no cycles. The bundler cannot keep module
// execution order across a cycle, which is why SvelteKit itself avoids
// manual chunks; a plan without them keeps it.
//
// The rest is left where the bundler puts it. A console page's own graph:
// grouping it by area was measured (0.19.1) and asked for more requests and
// more bytes, because areas share modules in more combinations than routes
// do. A module only an `import()` reaches (pdf.js, posthog, better-auth's
// client), so it is still fetched when asked for rather than with the page.
// And a module only the signed-out screens reach, so the sign-in screen's own
// graph stays what `authPreloads` lists.

/** The part of the bundler's module graph the plan reads. */
export interface ModuleGraph {
	readonly ids: Iterable<string>;
	/** The modules `id` imports with a static `import`, never `import()`. */
	readonly staticImports: (id: string) => readonly string[];
	readonly isEntry: (id: string) => boolean;
}

const NODE = /\/\.svelte-kit\/generated\/client(?:-optimized)?\/nodes\/(\d+)\.js$/;
const AUTH_LAYOUT = /\/src\/routes\/\(auth\)\/\+layout\.[^/]+$/;
const AUTH_SCREEN = /\/src\/routes\/\(auth\)\/(.+)\/\+page\.[^/]+$/;
const CONSOLE_FRAME = /\/src\/routes\/\(console\)\/\+(?:layout|error)\.[^/]+$/;

const reach = (graph: ModuleGraph, from: Iterable<string>): Set<string> => {
	const seen = new Set<string>();
	const pending = [...from];
	for (let id = pending.pop(); id !== undefined; id = pending.pop()) {
		if (!seen.has(id)) {
			seen.add(id);
			pending.push(...graph.staticImports(id));
		}
	}
	return seen;
};

/** The chunk each grouped module is written into. A module the plan does not
 *  name is left to the bundler. */
export function planChunks(graph: ModuleGraph): Map<string, string> {
	const ids = [...graph.ids];
	// SvelteKit hands every route node to the bundler as an entry of its own;
	// the shell's entries are the other two. Each node's one static import
	// under `src/routes/` is its route file.
	const nodes = ids
		.filter((id) => NODE.test(id))
		.map((id) => ({
			id,
			number: Number(NODE.exec(id)?.[1]),
			route: graph.staticImports(id).find((each) => each.includes('/src/routes/')) ?? ''
		}));
	// Nodes 0 and 1 are always the root layout and the root error page.
	const shell = reach(graph, [
		...ids.filter((id) => graph.isEntry(id) && !NODE.test(id)),
		...nodes.filter((node) => node.number <= 1).map((node) => node.id)
	]);
	const signedIn = reach(
		graph,
		nodes.filter((node) => node.route.includes('/src/routes/(console)/')).map((node) => node.id)
	);
	const frame = reach(
		graph,
		nodes.filter((node) => CONSOLE_FRAME.test(node.route)).map((node) => node.id)
	);
	const everyScreen = reach(
		graph,
		nodes.filter((node) => AUTH_LAYOUT.test(node.route)).map((node) => node.id)
	);
	const screens = new Map<string, string[]>();
	for (const node of nodes) {
		const screen = AUTH_SCREEN.exec(node.route)?.[1];
		if (screen === undefined) {
			continue;
		}
		for (const id of reach(graph, [node.id])) {
			screens.set(id, [...(screens.get(id) ?? []), screen]);
		}
	}

	const plan = new Map<string, string>();
	for (const id of new Set([...shell, ...reach(graph, nodes.map((node) => node.id))])) {
		if (graph.isEntry(id)) {
			continue;
		}
		const signedOut = everyScreen.has(id) || screens.has(id);
		const group = shell.has(id)
			? 'shell'
			: signedOut && signedIn.has(id)
				? `shared:${everyScreen.has(id) ? 'layout' : [...new Set(screens.get(id))].sort().join('+')}`
				: frame.has(id)
					? 'console'
					: undefined;
		if (group !== undefined) {
			plan.set(id, group);
		}
	}
	return plan;
}
