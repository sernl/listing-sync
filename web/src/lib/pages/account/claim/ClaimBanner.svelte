<script lang="ts">
	import Banner from '$lib/Banner.svelte';
	import './claim.css';

	// Dismissed for the visit and not remembered, deliberately. The durable
	// record of a seller declining is `slug_deferred` on the server, and
	// writing a second one into this browser would hide the prompt for good on
	// this machine and on no other.
	let { onDismiss }: { onDismiss: () => void } = $props();
</script>

<div class="claim-banner">
	<Banner tone="info" title="Your organisation has no name yet" {onDismiss}>
		{#snippet action()}
			<a class="btn small" href="/account/claim">Choose a name</a>
		{/snippet}
		Choose the name your organisation is known by. It is yours alone and appears in your address.
	</Banner>
</div>
