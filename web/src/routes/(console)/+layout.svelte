<script lang="ts">
	import '../../app.css';
	import { QueryClientProvider } from '@tanstack/svelte-query';
	import { page } from '$app/state';
	import { goto, invalidateAll } from '$app/navigation';
	import { createQuery } from '@tanstack/svelte-query';
	import { impersonationState } from '$lib/admin';
	import { impersonatedSession } from '$lib/auth-client';
	import Button from '$lib/Button.svelte';
	import Console from '$lib/Console.svelte';
	import ClaimBanner from '$lib/pages/account/claim/ClaimBanner.svelte';
	import ClaimScreen from '$lib/pages/account/claim/ClaimScreen.svelte';
	import { consoleGate } from '$lib/pages/account/claim/state';
	import { PUBLIC_ROUTES, signInHref, signedOutView } from '$lib/nav';
	import { createQueryClient, queryKeys } from '$lib/query';
	import { signOut } from '$lib/sign-out';
	import { resetIdentity } from '$lib/posthog';
	import { probeReachability } from '$lib/unreachable';
	import MaintenanceCard from '$lib/MaintenanceCard.svelte';
	import Toasts from '$lib/Toasts.svelte';
	import TermsConsentSheet from '$lib/TermsConsentSheet.svelte';
	import SuspendedNotice from '$lib/SuspendedNotice.svelte';
	import { activeSeason, siteGate } from '$lib/site';

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
	// The address asked for rides along as `next`, so a link into the console
	// -- a guide, a marketplace card -- lands on that page after signing in
	// rather than on the home page.
	const signIn = $derived(signInHref(page.url.pathname, page.url.search));
	$effect(() => {
		if (!data.session && !data.unreachable && !data.suspended && signedOut === 'redirecting') {
			goto(signIn);
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

	// The second look the reachability card shows, taken once the card is up.
	let probed = $state<string | null>(null);
	$effect(() => {
		if (data.unreachable) {
			void probeReachability().then((facts) => (probed = facts));
		}
	});
	const verdict = $derived(
		consoleGate({
			prompt: data.session?.slug_prompt,
			impersonating: impersonationState(identity.data ?? null) !== null,
			publicRoute: PUBLIC_ROUTES.includes(page.url.pathname),
			bannerDismissed
		})
	);

	// Maintenance mode, as the server already applies it to every page load:
	// this covers the navigations that never reach the server, such as a
	// seller signing in on `/login` and being taken into the console.
	const gate = $derived(
		siteGate({ site: data.site, operator: data.operator, pathname: page.url.pathname })
	);

	async function claimed() {
		await invalidateAll();
	}

	async function logout() {
		// Before the sign-out, so the identity is dropped even if the call
		// fails: the next seller on a shared machine must not inherit it.
		resetIdentity();
		await signOut(queryClient);
	}
</script>

<QueryClientProvider client={queryClient}>
	{#if data.suspended}
		<!-- The organisation is banned. Ahead of the sign-in redirect, which
		     would only end at the same refusal and so loop. -->
		<div class="auth">
			<div class="wordmark">
				<img src="/brand/logo.svg" alt="Teachouse" width="220" />
			</div>
			<div class="auth-card">
				<SuspendedNotice onSignOut={logout} />
			</div>
		</div>
	{:else if data.unreachable}
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
				<h1>This page could not load</h1>
				<p>{data.unreachable.sentence}</p>
				<Button tier="primary" onclick={() => location.reload()}>Try again</Button>
				<!-- The facts for the report, in small type: what was thrown and
				     what a second look found. A seller sends a screenshot of this
				     card, and the sentence alone could not tell a blocked fetch
				     from a page served off the device's cache. -->
				<p class="unreachable-detail">
					{data.unreachable.detail}
					{#if probed !== null}
						· {probed}
					{/if}
				</p>
			</div>
		</div>
	{:else if gate === 'maintenance'}
		<!-- Maintenance mode, which an operator switched on from Admin, Site.
		     Ahead of the console, so nothing below mounts and asks the API for
		     anything while the site is being worked on. -->
		<div class="auth">
			<div class="wordmark">
				<img src="/brand/logo.svg" alt="Teachouse" width="220" />
			</div>
			<MaintenanceCard
				message={data.site?.maintenance.message}
				onSignOut={data.session ? logout : undefined}
			/>
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
		<Console onLogout={logout} season={activeSeason(data.site)}>
			{#if verdict === 'banner'}
				<ClaimBanner onDismiss={() => (bannerDismissed = true)} />
			{/if}
			{@render children()}
		</Console>
		<!-- The terms asked for again when they have changed. Held until the
		     identity session is read, so an impersonating operator is never
		     shown a seller's agreement to make. -->
		<TermsConsentSheet
			suppressed={identity.isPending || impersonationState(identity.data ?? null) !== null}
		/>
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
				<h1>Taking you to sign in</h1>
				<p>Sign in to see this page.</p>
				<Button href={signIn} tier="primary">Sign in</Button>
			</div>
		</div>
	{/if}
	<Toasts />
</QueryClientProvider>
