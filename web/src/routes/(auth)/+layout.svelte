<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import type { SiteView } from '$lib/api';
	import AuthFrame from '$lib/AuthFrame.svelte';
	import { request } from '$lib/http';
	import { onFirstInteraction } from '$lib/idle';
	import MaintenanceCard from '$lib/MaintenanceCard.svelte';
	import { startTelemetry } from '$lib/posthog';
	import { siteGate } from '$lib/site';
	import Toasts from '$lib/Toasts.svelte';

	// Sign in, sign up and reset: the first screens a teacher sees, drawn
	// before anything is asked of the API. Nothing here imports the console's
	// shell, its query client, its icon-heavy chrome or its stylesheet; the
	// console layout loads those on the way in, after sign-in.
	let { children } = $props();

	// Maintenance mode, decided after the first paint rather than before it.
	// `tam-server` already answers a full page load of a closed page with its
	// maintenance page, so this covers the navigations that never reach it --
	// from sign in to sign up, say. A switch that cannot be read leaves the
	// page open, as the console's own gate does, and the operator question is
	// asked only when maintenance is on, because only then does it change what
	// is drawn.
	let site = $state<SiteView | null>(null);
	let operator = $state(false);
	const gate = $derived(siteGate({ site, operator, pathname: page.url.pathname }));

	onMount(() => {
		void request<SiteView>('/v1/site').then(
			(read) => {
				site = read;
				if (read.maintenance.on) {
					void request<SiteView>('/v1/admin/site').then(
						() => (operator = true),
						() => undefined
					);
				}
			},
			() => undefined
		);
		// Product analytics waits for the visitor to do something: a visit that
		// only looks at the sign-in card fetches none of it.
		return onFirstInteraction(window, startTelemetry);
	});
</script>

<AuthFrame>
	{#if gate === 'maintenance'}
		<MaintenanceCard message={site?.maintenance.message} />
	{:else}
		{@render children()}
	{/if}
</AuthFrame>
<Toasts />
