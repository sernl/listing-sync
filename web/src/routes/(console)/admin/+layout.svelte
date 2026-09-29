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
		<p class="quiet">Checking admin access…</p>
	</div>
{:else if reason === 'unconfigured'}
	<div class="page">
		<Placeholder
			icon="layout-dashboard"
			headline="Admin pages are off on this server"
			body="This server runs without a backoffice database. Nothing is wrong with your account."
		/>
	</div>
{:else if reason === 'not-an-operator'}
	<div class="page">
		<Placeholder
			icon="layout-dashboard"
			headline="This account is not an admin"
			body="Admin access is set by hand on the server, not in the app. If you need it, ask
				the person who runs Teachouse."
		/>
	</div>
{:else}
	<div class="page">
		<Placeholder
			icon="layout-dashboard"
			headline="We could not load the admin pages"
			body="Try reloading the page."
		/>
	</div>
{/if}
