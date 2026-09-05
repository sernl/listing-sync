<script lang="ts">
	import '../app.css';
	import { QueryClientProvider } from '@tanstack/svelte-query';
	import { page } from '$app/state';
	import { goto, invalidateAll } from '$app/navigation';
	import { createQuery } from '@tanstack/svelte-query';
	import { impersonationState } from '$lib/admin';
	import { impersonatedSession, signOutEverywhere } from '$lib/auth-client';
	import Console from '$lib/Console.svelte';
	import ClaimBanner from '$lib/pages/account/claim/ClaimBanner.svelte';
	import ClaimScreen from '$lib/pages/account/claim/ClaimScreen.svelte';
	import { consoleGate } from '$lib/pages/account/claim/state';
	import { createQueryClient, queryKeys } from '$lib/query';
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

	// The organisation's slug, answered by the same `/v1/whoami` this layout
	// already loads, so the gate costs no request and is decided before the
	// console renders rather than after it has flashed.
	//
	// The identity session is read here as well as in `Console`, under the same
	// query key, so the two share one request rather than making two. It is
	// what tells the gate that an operator is impersonating -- see `consoleGate`
	// for why that case must not be gated.
	//
	// The client is handed over rather than read from context. This query is in
	// the same component that renders `QueryClientProvider` below, and a
	// provider publishes its client to its children: a parent cannot read the
	// context it provides, so reading it here throws during layout setup and
	// takes the whole console with it.
	const identity = createQuery(
		() => ({
			queryKey: queryKeys.identitySession,
			queryFn: () => impersonatedSession()
		}),
		() => queryClient
	);

	let bannerDismissed = $state(false);
	const verdict = $derived(
		consoleGate({
			prompt: data.session?.slug_prompt,
			impersonating: impersonationState(identity.data ?? null) !== null,
			publicRoute: PUBLIC.includes(page.url.pathname),
			bannerDismissed
		})
	);

	async function claimed() {
		await invalidateAll();
	}

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
	{#if data.session && verdict === 'claim-screen'}
		<div class="auth">
			<div class="wordmark">
				<img class="leaf" src="/favicon.svg" alt="" width="28" height="28" /> Teachouse
			</div>
			<ClaimScreen onDone={claimed} />
			<!-- A gate with no way out is a trap: this is the only screen a newly
			     provisioned seller can reach, so signing out has to be reachable
			     from it. -->
			<p class="claim-out"><button type="button" class="link" onclick={logout}>Sign out</button></p>
		</div>
	{:else if data.session}
		<Console onLogout={logout}>
			{#if verdict === 'banner'}
				<ClaimBanner onDismiss={() => (bannerDismissed = true)} />
			{/if}
			{@render children()}
		</Console>
	{:else}
		<div class="auth">
			<div class="wordmark">
				<img class="leaf" src="/favicon.svg" alt="" width="28" height="28" /> Teachouse
			</div>
			{@render children()}
		</div>
	{/if}
	<div class="toasts">
		{#each $toastStore as entry (entry.id)}
			<div class="toast {entry.tone === 'error' ? 'error' : ''}">{entry.message}</div>
		{/each}
	</div>
</QueryClientProvider>
