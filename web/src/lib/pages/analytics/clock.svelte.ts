/** A clock the page can read reactively.
 *
 * Every age on this page — the header's freshness, each tile's "as at", each
 * row's capture age — is a function of the current instant, and `Date.now()`
 * read inside a `$derived` is not a dependency of it: the deriveds recompute
 * when a query result changes and at no other time, so the ages freeze at
 * whatever they were on first render. That is the exact inversion the page
 * promises it prevents, and a console that is also shipped as a desktop and an
 * Android app cannot rely on a window-focus refetch to hide it.
 *
 * Thirty seconds because `agoLabel` resolves to whole minutes, so a tick any
 * slower can leave a minute-old figure reading "just now".
 */
export function ticker(period = 30_000) {
	let now = $state(Date.now());
	$effect(() => {
		const id = setInterval(() => {
			now = Date.now();
		}, period);
		return () => clearInterval(id);
	});
	return {
		get now() {
			return now;
		}
	};
}
