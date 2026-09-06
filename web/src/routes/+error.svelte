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
</script>

<div class="page">
	<div class="auth-card">
		<h1>{missing ? 'That page is not here' : 'That page could not be opened'}</h1>
		<p>
			{#if missing}
				The address does not match anything in Teachouse. It may have been a typed slip, or a
				link to something that has since moved.
			{:else}
				Something went wrong while opening it. Opening it again usually clears it.
			{/if}
		</p>
		<!-- One destination, not a branch on the session: a signed-out visitor
		     never reaches this page, because the root layout sends a session-less
		     visit to any address it does not list as public to `/login` and
		     renders its own interstitial in place of this one while it goes. -->
		<Button href="/resources" tier="primary">Back to your resources</Button>
	</div>
</div>
