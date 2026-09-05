<script lang="ts">
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import './claim.css';

	let { onDismiss }: { onDismiss: () => void } = $props();
</script>

<div class="claim-banner">
	<Banner tone="info" title="Your organisation has no name yet">
		{#snippet action()}
			<a class="btn small" href="/account/claim">Choose a name</a>
			<Button tier="quiet" small onclick={onDismiss}>Dismiss</Button>
		{/snippet}
		Choose the name your organisation is known by. It is yours alone and appears in your address.
	</Banner>
</div>
