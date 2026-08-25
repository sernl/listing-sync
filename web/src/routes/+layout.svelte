<script lang="ts">
	import '../app.css';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { api } from '$lib/api';
	import { toastStore } from '$lib/toast';
	import { invalidateAll } from '$app/navigation';

	let { data, children } = $props();

	const PUBLIC = ['/login', '/status'];
	$effect(() => {
		if (!data.session && !PUBLIC.includes(page.url.pathname)) {
			goto('/login');
		}
	});

	async function logout() {
		await api.logout();
		await invalidateAll();
		goto('/login');
	}
</script>

<div class="min-h-screen bg-slate-50 text-slate-900">
	<header class="border-b border-slate-200 bg-white">
		<nav class="mx-auto flex max-w-6xl items-center gap-6 px-4 py-3">
			<span class="font-semibold">listing-sync</span>
			{#if data.session}
				<a class="hover:underline" href="/">Products</a>
				<a class="hover:underline" href="/jobs">Jobs</a>
				<a class="hover:underline" href="/connections">Connections</a>
				<a class="hover:underline" href="/queue">Reconciliation</a>
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
