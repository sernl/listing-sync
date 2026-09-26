<script lang="ts">
	import { page } from '$app/state';
	import Button from '$lib/Button.svelte';

	// SvelteKit's own fallback draws the status at 3rem beside the raw message
	// on a bare white page, with no wordmark and nothing to press. This replaces
	// it for everything the root layout survives: an address the router does not
	// match, and any page load that failed. A root-layout load failure is the
	// one case this cannot catch, because the layout that renders it is what
	// failed; `src/error.html` is that case.
	//
	// No class of its own: `.auth-card` inside `.page` reads correctly in both
	// frames this can render in — the signed-out screen, which supplies `.auth`
	// and the wordmark around it, and the console, which supplies its own shell.
	const missing = $derived(page.status === 404);
	/** What the load threw, in the words it carried. Shown for the reason
	 *  `render-failure.ts` gives: a phone has no devtools pane, and a sentence
	 *  the seller can read out is the difference between a bug we can find
	 *  and one we cannot. */
	const cause = $derived(missing ? null : (page.error?.message?.trim() || null));
</script>

<div class="page">
	<div class="auth-card">
		<h1>{missing ? 'That page is not here' : 'That page could not be opened'}</h1>
		<p>
			{#if missing}
				We could not find this page. Check the address, or the page may have moved.
			{:else}
				Something went wrong. Try opening it again.
			{/if}
		</p>
		{#if cause !== null}
			<p class="drew-why">{cause}</p>
		{/if}
		<!-- One destination, not a branch on the session: a signed-out visitor
		     never reaches this page, because the root layout sends a session-less
		     visit to any address it does not list as public to `/login` and
		     renders its own interstitial in place of this one while it goes. -->
		<Button href="/resources" tier="primary">Back to your resources</Button>
	</div>
</div>

<style>
	.drew-why {
		margin: 0 0 12px;
		color: var(--muted);
		font-size: 12px;
		line-height: 1.5;
		overflow-wrap: break-word;
	}
</style>
