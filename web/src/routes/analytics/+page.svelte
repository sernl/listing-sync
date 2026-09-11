<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import Button from '$lib/Button.svelte';
	import { entitlementRead, featureOf } from '$lib/entitlement-read';
	import PageHead from '$lib/PageHead.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import Analytics from '$lib/pages/analytics/Analytics.svelte';

	// The gate sits on the route rather than inside the screen, so a plan
	// without analytics never mounts it: that component walks every page of
	// the catalogue and every mapping to build its figures, and a screen that
	// fetched all of it and then refused to draw would spend a seller's
	// bandwidth to tell them they cannot see it.
	const plan = createQuery(() => entitlementRead);
	const refusal = $derived(featureOf(plan.data, 'analytics'));
</script>

{#if refusal === null}
	<Analytics />
{:else}
	<div class="page">
		<PageHead
			icon="chart-line"
			title="Analytics"
			description="What each marketplace reports about your resources."
		/>
		<Placeholder icon="chart-line" headline="Not on your plan" body={refusal}>
			{#snippet actions()}
				<Button tier="primary" href="/settings/subscription">See plans</Button>
			{/snippet}
		</Placeholder>
	</div>
{/if}
