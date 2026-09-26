<script lang="ts">
	import { goto, invalidateAll } from '$app/navigation';
	import ClaimScreen from '$lib/pages/account/claim/ClaimScreen.svelte';

	// Reached by an organisation that predates the slug, from the banner the
	// console shows it. A newly provisioned one never navigates here: the layout
	// renders the same screen in place of the console, because a gate the seller
	// can navigate away from is not a gate.
	async function done() {
		await invalidateAll();
		await goto('/');
	}
</script>

<svelte:head><title>Name your account</title></svelte:head>

<ClaimScreen dismissable onDone={done} />
