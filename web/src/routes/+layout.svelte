<script lang="ts">
	import '../app.css';
	import { QueryClientProvider } from '@tanstack/svelte-query';
	import { page } from '$app/state';
	import { goto, invalidateAll } from '$app/navigation';
	import { signOutEverywhere } from '$lib/auth-client';
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
	<div class="min-h-screen bg-slate-50 text-slate-900">
		<header class="border-b border-slate-200 bg-white">
			<nav class="mx-auto flex max-w-6xl items-center gap-6 px-4 py-3">
				<span class="font-semibold">listing-sync</span>
				{#if data.session}
					<a class="hover:underline" href="/">Products</a>
					<a class="hover:underline" href="/jobs">Jobs</a>
					<a class="hover:underline" href="/connections">Connections</a>
					<a class="hover:underline" href="/queue">Reconciliation</a>
					<a class="hover:underline" href="/settings">Settings</a>
				{/if}
				<a class="hover:underline" href="/status">Status</a>
				<span class="grow"></span>
				{#if data.session}
					<button class="text-sm text-slate-500 hover:underline" onclick={logout}>
						Log out
					</button>
				{/if}
			</nav>
		</header>
		<main class="mx-auto max-w-6xl px-4 py-6">
			{@render children()}
		</main>
		<div class="fixed right-4 bottom-4 flex flex-col gap-2">
			{#each $toastStore as entry (entry.id)}
				<div
					class="rounded px-4 py-2 text-sm text-white shadow
						{entry.tone === 'error' ? 'bg-red-600' : 'bg-slate-800'}"
				>
					{entry.message}
				</div>
			{/each}
		</div>
	</div>
</QueryClientProvider>
