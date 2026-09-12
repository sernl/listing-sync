<script lang="ts">
	import { tick } from 'svelte';
	import '../app.css';
	import { QueryClientProvider } from '@tanstack/svelte-query';
	import { page } from '$app/state';
	import { goto, invalidateAll } from '$app/navigation';
	import { createQuery } from '@tanstack/svelte-query';
	import { impersonationState } from '$lib/admin';
	import { impersonatedSession } from '$lib/auth-client';
	import Button from '$lib/Button.svelte';
	import Console from '$lib/Console.svelte';
	import Icon from '$lib/Icon.svelte';
	import ClaimBanner from '$lib/pages/account/claim/ClaimBanner.svelte';
	import ClaimScreen from '$lib/pages/account/claim/ClaimScreen.svelte';
	import { consoleGate } from '$lib/pages/account/claim/state';
	import { PUBLIC_ROUTES, signedOutView } from '$lib/nav';
	import { createQueryClient, queryKeys } from '$lib/query';
	import { signOut } from '$lib/sign-out';
	import { afterToastDismissed, captureSlots, focusRegion } from '$lib/focus-return';
	import type { IconName } from '$lib/icons';
	import { dismiss, sweep, toastStore, type Toast } from '$lib/toast';

	let { data, children } = $props();

	// One client for the life of the app: the layout outlives every navigation,
	// so the cache does too.
	const queryClient = createQueryClient();

	// What a browser with no API session is shown here, from `$lib/nav`, which is
	// where the rest of this console's route knowledge lives. The branch below
	// reads it as well as this effect: rendering a console page while the
	// navigation is still in flight mounts its markup and fires its queries for
	// a frame, against a session that is not there.
	const signedOut = $derived(signedOutView(page.url.pathname));
	$effect(() => {
		if (!data.session && !data.unreachable && signedOut === 'redirecting') {
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
			publicRoute: PUBLIC_ROUTES.includes(page.url.pathname),
			bannerDismissed
		})
	);

	async function claimed() {
		await invalidateAll();
	}

	async function logout() {
		await signOut(queryClient);
	}

	// The console's notice glyph for each tone, as `Banner` holds its own.
	// Here rather than in `toast.ts`, which is a pure store with no view in
	// it, and read through the record rather than written as a ternary in the
	// markup: `icons.test.ts` sweeps the quoted arms of an `Icon name={...}`
	// expression, so a tone compared inline reads to it as an icon name.
	const TOAST_GLYPH: Record<Toast['tone'], IconName> = {
		info: 'circle-check',
		error: 'circle-alert'
	};

	// The toast stack's clock lives here rather than in the store, because what
	// pauses it is a pointer and a focus ring, and only the element knows about
	// those. A sweep that removed the toast under the pointer would steal the
	// click that was about to close it, so the stack holds every standing toast
	// while it is hovered or focused within, and lets them go on the way out.
	//
	// The interval is armed only while something is standing, and `sweep` is
	// silent when it drops nothing, so an idle console redraws nothing.
	let held = $state(false);

	$effect(() => {
		if (held || $toastStore.length === 0) {
			return;
		}
		const timer = setInterval(() => sweep(Date.now()), 250);
		return () => clearInterval(timer);
	});

	// Who was working when each toast arrived, keyed by that toast's id.
	// Captured as the toast appears rather than as it is closed, because by then
	// the close control itself holds focus and the answer is gone. Per toast
	// rather than one slot for the stack: two overlapping toasts interrupted two
	// different controls, and a single slot sent the second one's closer back to
	// whatever the first had interrupted.
	//
	// Held here rather than on the toast record, so `toast.ts` stays a pure
	// store with no element references in it. Not `$state`: nothing renders from
	// it, and a reactive read here would re-run the effect that writes it.
	const interrupted = new Map<number, HTMLElement | null>();

	$effect(() => {
		const { add, drop } = captureSlots(
			$toastStore.map((entry) => entry.id),
			[...interrupted.keys()]
		);
		for (const id of drop) {
			interrupted.delete(id);
		}
		if (add.length === 0) {
			return;
		}
		const active = document.activeElement;
		const opener =
			active instanceof HTMLElement && active.closest('.toasts') === null ? active : null;
		for (const id of add) {
			interrupted.set(id, opener);
		}
	});

	function keyedToast(event: KeyboardEvent, id: number) {
		if (event.key !== 'Escape') {
			return;
		}
		event.stopPropagation();
		void closeToast(id);
	}

	async function closeToast(id: number) {
		const previous = interrupted.get(id) ?? null;
		dismiss(id);
		await tick();
		const region = document.querySelector('main');
		switch (
			afterToastDismissed({
				previous: previous?.isConnected === true,
				region: region !== null
			})
		) {
			case 'previous':
				previous?.focus();
				break;
			case 'region':
				if (region !== null) {
					focusRegion(region);
				}
				break;
			case 'none':
				break;
		}
	}
</script>

<QueryClientProvider client={queryClient}>
	{#if data.unreachable}
		<!-- The first request got no Teachouse answer. Ahead of every other
		     branch, because nothing below can be trusted: there is no session
		     to render and no proof there is none. The sentence names what
		     actually came back, which is the one thing the old
		     `error.html` fallback could not say. -->
		<div class="auth">
			<div class="wordmark">
				<img src="/brand/logo.svg" alt="Teachouse" width="220" />
			</div>
			<div class="auth-card">
				<h1>Teachouse could not be reached</h1>
				<p>{data.unreachable.sentence}</p>
				<Button tier="primary" onclick={() => location.reload()}>Try again</Button>
			</div>
		</div>
	{:else if data.session && verdict === 'claim-screen'}
		<div class="auth">
			<div class="wordmark">
				<img src="/brand/logo.svg" alt="Teachouse" width="220" />
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
	{:else if signedOut === 'public'}
		<div class="auth">
			<div class="wordmark">
				<img src="/brand/logo.svg" alt="Teachouse" width="220" />
			</div>
			{@render children()}
		</div>
	{:else}
		<!-- A console address reached without a session. The effect above is
		     already navigating to `/login`; this is what stands while it does,
		     in place of the page's own markup, which would otherwise mount and
		     fire every query on it against a session that is not there. -->
		<div class="auth">
			<div class="wordmark">
				<img src="/brand/logo.svg" alt="Teachouse" width="220" />
			</div>
			<div class="auth-card">
				<h1>Signing you in</h1>
				<p>This page needs a Teachouse account. Taking you to the sign-in screen.</p>
				<Button href="/login" tier="primary">Sign in</Button>
			</div>
		</div>
	{/if}
	<div
		class="toasts"
		role="status"
		aria-live="polite"
		onmouseenter={() => (held = true)}
		onmouseleave={() => (held = false)}
		onfocusin={() => (held = true)}
		onfocusout={() => (held = false)}
	>
		{#each $toastStore as entry (entry.id)}
			<!-- Escape closes the toast focus is inside, and the handler sits on
			     that toast rather than on the window so a dialog open over the
			     stack keeps the key. The close button is the only thing in a toast
			     that takes focus, so this reaches the same seller by another
			     press; the element is not a control in its own right. -->
			<!-- svelte-ignore a11y_no_static_element_interactions -->
			<div
				class="toast {entry.tone === 'error' ? 'error' : ''}"
				onkeydown={(event) => keyedToast(event, entry.id)}
			>
				<span class="toast-mark">
					<Icon name={TOAST_GLYPH[entry.tone]} size={18} />
				</span>
				<span class="toast-say">{entry.message}</span>
				<button
					type="button"
					class="toast-close"
					aria-label="Close: {entry.message}"
					onclick={() => closeToast(entry.id)}
				>
					<Icon name="x" size={16} />
				</button>
			</div>
		{/each}
	</div>
</QueryClientProvider>
