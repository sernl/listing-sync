<script lang="ts">
	import '../app.css';
	import { QueryClientProvider } from '@tanstack/svelte-query';
	import { page } from '$app/state';
	import { goto, invalidateAll } from '$app/navigation';
	import { signOutEverywhere } from '$lib/auth-client';
	import Console from '$lib/Console.svelte';
	import { createQueryClient } from '$lib/query';
	import { toast, toastStore } from '$lib/toast';

	let { data, children } = $props();

	// One client for the life of the app: the layout outlives every navigation,
	// so the cache does too.
	const queryClient = createQueryClient();

	// The pages reachable without an API session. `/status` is deliberately
	// among them: it matters most when signing in is what is broken.
	const PUBLIC = ['/login', '/signup', '/reset', '/reset/confirm', '/status'];
	$effect(() => {
		if (!data.session && !PUBLIC.includes(page.url.pathname)) {
			goto('/login');
		}
	});

	async function logout() {
		const complete = await signOutEverywhere();
		if (!complete) {
			toast('error', 'Signed out here, but one of the two sessions may still be open.');
		}
		queryClient.clear();
		await invalidateAll();
		await goto('/login');
	}
</script>

<QueryClientProvider client={queryClient}>
	{#if data.session}
		<Console onLogout={logout}>{@render children()}</Console>
	{:else}
		<div class="auth">
			<div class="wordmark"><span class="leaf" aria-hidden="true">T</span> Teachouse</div>
			{@render children()}
		</div>
	{/if}
	<div class="toasts">
		{#each $toastStore as entry (entry.id)}
			<div class="toast {entry.tone === 'error' ? 'error' : ''}">{entry.message}</div>
		{/each}
	</div>
</QueryClientProvider>
