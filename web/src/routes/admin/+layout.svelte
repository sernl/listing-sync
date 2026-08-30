<script lang="ts">
	import type { Snippet } from 'svelte';
	import { createQuery } from '@tanstack/svelte-query';
	import { operatorVerdict, outsiderReason } from '$lib/admin';
	import { api } from '$lib/api';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';

	let { children }: { children: Snippet } = $props();

	// The same probe the sidebar reads, under the same key, so navigating to
	// `/admin` by URL asks nothing the shell has not already asked.
	const probe = createQuery(() => ({
		queryKey: queryKeys.operator,
		queryFn: () => api.adminSyncHealth(),
		retry: false,
		staleTime: Infinity,
		gcTime: Infinity
	}));

	const verdict = $derived(
		operatorVerdict({ answered: probe.data !== undefined, failure: probe.error ?? null })
	);
	const reason = $derived(verdict === 'outsider' ? outsiderReason(probe.error) : null);
</script>

{#if verdict === 'operator'}
	{@render children()}
{:else if verdict === 'checking'}
	<div class="page">
		<p class="quiet">Checking whether this account operates the platform…</p>
	</div>
{:else if reason === 'unconfigured'}
	<div class="page">
		<Placeholder
			icon="◈"
			headline="This deployment serves no operator surface"
			body="It was started without a backoffice database, so it serves none of it rather
				than half of it. Nothing is wrong with your account."
		/>
	</div>
{:else if reason === 'not-an-operator'}
	<div class="page">
		<Placeholder
			icon="◈"
			headline="This account does not operate the platform"
			body="The operator marking is granted on the box by hand and is separate from
				anything inside the product. If you should have it, ask the person who runs
				the deployment."
		/>
	</div>
{:else}
	<div class="page">
		<Placeholder
			icon="◈"
			headline="The operator surface could not be read"
			body="The request did not come back with an answer we can act on. Reloading is
				the only thing worth trying from here."
		/>
	</div>
{/if}
