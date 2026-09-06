<script lang="ts">
	import type { Snippet } from 'svelte';
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { page } from '$app/state';
	import { goto, invalidateAll } from '$app/navigation';
	import { impersonationState, operatorVerdict } from '$lib/admin';
	import { api } from '$lib/api';
	import { impersonatedSession, stopImpersonatingAndRestore } from '$lib/auth-client';
	import { present, type StatusPresentation } from '$lib/connection-status';
	import { desktopInvoker, registerThisMachine } from '$lib/desktop';
	import { renderFailureReport } from '$lib/render-failure';
	import Icon from '$lib/Icon.svelte';
	import ImpersonationBanner from '$lib/ImpersonationBanner.svelte';
	import {
		ADMIN_SECTION,
		CREATE_TAB,
		HOME_ITEM,
		PHONE_BAR,
		SECTIONS,
		breadcrumbFor,
		currentDestination,
		initialsOf,
		sectionFor
	} from '$lib/nav';
	import { queryKeys } from '$lib/query';
	import SearchPalette from '$lib/SearchPalette.svelte';
	import { opensPalette } from '$lib/search-palette';

	let { children, onLogout }: { children: Snippet; onLogout: () => void } = $props();

	const organisation = createQuery(() => ({ queryKey: queryKeys.org, queryFn: () => api.org() }));
	const connections = createQuery(() => ({
		queryKey: queryKeys.connections,
		queryFn: () => api.connections()
	}));
	// The operator probe: one read of the cheapest operator route, cached for
	// the session. A 401 is the deliberate blank refusal every non-operator
	// gets, so it is not retried and never surfaces as a fault -- it is simply
	// how this client learns the human is not an operator.
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

	// The identity session, read rather than remembered: a reload mid
	// impersonation must raise the banner from what it finds.
	const identity = createQuery(() => ({
		queryKey: queryKeys.identitySession,
		queryFn: () => impersonatedSession()
	}));
	const impersonation = $derived(impersonationState(identity.data ?? null));

	const queryClient = useQueryClient();

	// The console registers the machine it is running on, once per load.
	//
	// Here rather than on the machines page because this shell is what every
	// signed-in page renders inside, and a machine that only registered when
	// the seller happened to open Marketplaces would be missing from the list
	// the rest of the time. It is also the one place that knows a seller is
	// signed in: the layout renders this component only under a session, and a
	// session is exactly what the application's own start-up check-in lacks --
	// that runs before the sign-in, and then not again for an hour on a
	// computer or until the next resume on a phone.
	//
	// Once, not on every render: an `$effect` whose body reads nothing reactive
	// runs on mount alone. In a browser there is no invoker, the call answers
	// false, and nothing is refetched.
	$effect(() => {
		void registerThisMachine(desktopInvoker()).then((registered) => {
			if (registered) {
				// Only the device list, and only when a row may have appeared: the
				// machines page refreshes if it is open, and nothing else is
				// disturbed.
				void queryClient.invalidateQueries({ queryKey: queryKeys.devices });
			}
		});
	});

	let stopping = $state(false);
	let stopRefusal = $state<string | null>(null);

	async function stopImpersonating() {
		stopping = true;
		stopRefusal = null;
		try {
			await stopImpersonatingAndRestore();
			// Everything cached was read as the impersonated tenant. Clearing is
			// not tidiness: a stale organisation left in the cache would render
			// somebody else's workspace under the operator's own session.
			queryClient.clear();
			await invalidateAll();
			await goto('/admin/users');
		} catch (failure) {
			// The banner stays up rather than being dismissed, and the identity
			// session is deliberately not re-read: a failure here can leave the two
			// planes disagreeing, and a banner that vanished over that would hide
			// the one state that must stay visible. Logging out ends both.
			const said =
				failure instanceof Error ? failure.message : 'The impersonation was not stopped.';
			stopRefusal = `${said} You are still signed in as them — log out to end both sessions.`;
		} finally {
			stopping = false;
		}
	}

	/** Records a page that failed to draw, so a blank region leaves a trace.
	 *
	 *  `console.error` because this console has no diagnostics channel of its
	 *  own yet; when one exists, this is the single place that changes. */
	function reportDrawFailure(error: unknown) {
		console.error(renderFailureReport(error, page.url.pathname));
	}

	const pathname = $derived(page.url.pathname);
	const crumb = $derived(breadcrumbFor(pathname));
	const orgName = $derived(organisation.data?.name);
	const initials = $derived(initialsOf(orgName));
	const links = $derived(connections.data ?? []);

	// Which section the rail lights and whose pages the card lists. The
	// operator's own section joins the rail only for a human the probe
	// admitted, and is asked for by name here so a non-operator on an `/admin`
	// path still lands on a section that exists rather than on a card of
	// destinations that answer 401.
	const operator = $derived(verdict === 'operator');
	const section = $derived(sectionFor(pathname, operator));
	// The destination this path belongs to, rather than a per-item `isCurrent`.
	// A page can be owned by an item it does not sit under — an import's detail
	// page lives at `/sync/requests/<id>` and belongs to Import — and only the
	// longest-claim answer gets that right without lighting two items elsewhere.
	const owning = $derived(currentDestination(pathname));
	// The card renders only for a section that has pages. The home path and the
	// open-questions queue belong to no section at all, and on those the rail
	// lights nothing rather than claiming one.
	const card = $derived(section !== null && section.items.length > 0 ? section : null);
	// Account sits at the foot of the rail beside Help, which is where the
	// founder's own sketch and Vendoo both put it; the rest run under the mark.
	const railSections = $derived([
		...SECTIONS.filter((entry) => entry.id !== 'account'),
		...(operator ? [ADMIN_SECTION] : [])
	]);
	const accountSection = $derived(SECTIONS.find((entry) => entry.id === 'account'));

	/** The dot beside a marketplace in the top bar. `checking` earns no colour
	 *  on purpose: it is linked and unverified, which is nothing to report. */
	const DOT: Record<StatusPresentation['tone'], string> = {
		ok: 'ok',
		mut: '',
		run: 'warn',
		bad: 'bad'
	};

	// The palette is the console's search now: it floats over whichever page
	// the seller is on and opens the resource itself, rather than sending them
	// to the board filtered down to it. The board keeps its own search field
	// for narrowing what it is already showing.
	let searching = $state(false);

	function shortcut(event: KeyboardEvent) {
		if (opensPalette(event)) {
			event.preventDefault();
			searching = true;
		}
	}
</script>

<svelte:window onkeydown={shortcut} />

<svelte:head><title>{crumb} · Teachouse</title></svelte:head>

<div class="app" class:no-nav={card === null}>
	{#if impersonation}
		<ImpersonationBanner
			who={impersonation.who}
			{stopping}
			refusal={stopRefusal}
			onStop={stopImpersonating}
		/>
	{/if}

	<nav class="rail" aria-label="Sections">
		<a class="mark" href={HOME_ITEM.href} aria-label={HOME_ITEM.label}>
			<img src="/favicon.svg" alt="" width="34" height="34" />
		</a>

		{#each railSections as entry (entry.id)}
			<a
				class="rail-item"
				href={entry.href}
				aria-current={entry.id === section?.id ? 'page' : undefined}
				aria-label={entry.label}
				title={entry.label}
			>
				<Icon name={entry.icon} size={20} />
			</a>
		{/each}

		<span class="rail-gap"></span>

		<a class="rail-item" href="/guides" aria-label="Help and guides" title="Help and guides">
			<Icon name="circle-question-mark" size={20} />
		</a>

		{#if accountSection}
			<a
				class="rail-item"
				href={accountSection.href}
				aria-current={accountSection.id === section?.id ? 'page' : undefined}
				aria-label={accountSection.label}
				title={accountSection.label}
			>
				<Icon name={accountSection.icon} size={20} />
			</a>
		{/if}
	</nav>

	{#if card}
		<div class="nav-card">
			<div class="nav-top">
				<h2>{card.label}</h2>
			</div>

			{#if card.primary}
				<a class="cta nav-primary" href={card.primary.href}>
					<Icon name={card.primary.icon} size={16} />
					{card.primary.label}
				</a>
			{/if}

			<!-- Named directly rather than by `aria-labelledby`: the title it would
			     point at is hidden below the tablet breakpoint, and whether a hidden
			     element still supplies a name is not something to depend on. -->
			<nav aria-label={card.label}>
				{#each card.items as item (item.href)}
					<a
						class="nav-item"
						href={item.href}
						aria-current={item.href === owning?.href ? 'page' : undefined}
					>
						<span class="ico"><Icon name={item.icon} size={16} /></span>
						{item.label}
						{#if item.soon}
							<span class="chip">soon</span>
						{/if}
					</a>
				{/each}
			</nav>

			{#if card.id === 'account'}
				<button class="nav-item nav-foot" type="button" onclick={onLogout}>
					<span class="ico"><Icon name="log-out" size={16} /></span>
					Log out
				</button>
			{/if}
		</div>
	{/if}

	<main class="region">
		<div class="top">
			<span class="conn">
				{#each links as link (link.id)}
					<span title={present(link.status).explanation}>
						<span class="dot {DOT[present(link.status).tone]}"></span>{link.marketplace}
						{link.status}
					</span>
				{/each}
			</span>
			<span class="grow"></span>
			<button class="search search-open" type="button" onclick={() => (searching = true)}>
				<Icon name="search" size={15} />
				<span class="search-said">Search resources…</span>
				<kbd>ctrl K</kbd>
			</button>
			<a class="cta" href={CREATE_TAB.href}>{CREATE_TAB.label}</a>
			<!-- Named here rather than by its contents. Below the phone breakpoint
			     `.account .who` is `display: none`, which takes its text out of the
			     accessibility tree as well as off the screen, and the avatar beside
			     it is `aria-hidden` because the initials are decoration. Without
			     this label the one surviving route to Account on a phone announces
			     as a link with no name at all.

			     `aria-current` for the same reason: Account is the one section with
			     no cell on the tab bar, so on the width where this is the whole of
			     it, the state every other section gets from its lit tab has to come
			     from here. -->
			<a
				class="account"
				href="/settings"
				aria-label={accountSection?.label ?? 'Account'}
				aria-current={accountSection !== undefined && accountSection.id === section?.id
					? 'page'
					: undefined}
			>
				<span class="avatar" aria-hidden="true">{initials}</span>
				<span class="who">
					<span class="org" title={orgName}>
						{#if orgName !== undefined}
							{orgName}
						{:else if organisation.isError}
							Organisation unavailable
						{:else}
							Loading…
						{/if}
					</span>
					<!-- The name the seller chose, in place of the host they are on:
					     the host told them nothing they decided. Rendered only once
					     the organisation has been read, because "no name yet" before
					     then is a claim about a row nobody has looked at. A newly
					     provisioned organisation never reaches this bar -- the claim
					     screen stands before it -- so the absent case here is only an
					     organisation that predates the slug, which the banner is
					     already asking about. -->
					<span class="plan">
						{#if organisation.data}{organisation.data.slug ?? 'No name yet'}{/if}
					</span>
				</span>
			</a>
		</div>

		<!-- A page that throws while drawing takes only its own region with it.
		     Without this the error escapes to the root and Svelte tears the whole
		     tree down, so the seller is left looking at nothing at all and has no
		     sentence to report; the shell around this boundary keeps drawing, so
		     the navigation still works and the failure is visibly local.

		     The failed arm is plain elements on the shell's own classes rather
		     than the components the rest of the console is built from. It renders
		     at the moment something has already gone wrong, and a component that
		     threw in here would escape to the root exactly as the page did. -->
		<!-- Keyed on the path so each route gets its own boundary. A boundary that
		     has tripped stays tripped until something recreates it, and this one
		     wraps every route, so before the key one page throwing left the
		     refusal standing over every page reached afterwards -- the seller saw
		     Marketplaces broken because Resources had been. The key costs no
		     remount that was not already happening: SvelteKit replaces the route
		     component on a path change regardless, and a query-only change, such
		     as the board's own filter, does not touch `pathname`. -->
		{#key pathname}
			<svelte:boundary onerror={(error) => reportDrawFailure(error)}>
				{@render children()}

				{#snippet failed()}
					<div class="page">
						<p>This page could not be drawn. Reload to try again.</p>
						<button class="btn" type="button" onclick={() => location.reload()}>Reload</button>
					</div>
				{/snippet}
			</svelte:boundary>
		{/key}
	</main>

	<nav class="tabbar" aria-label="Sections">
		<!-- Keyed by label rather than href: the search cell opens the palette in
		     place and so carries no destination at all. -->
		{#each PHONE_BAR as tab (tab.label)}
			{#if tab.search}
				<button class="tab-item" type="button" onclick={() => (searching = true)}>
					<span class="ico"><Icon name={tab.icon} size={19} /></span>
					<span>{tab.label}</span>
				</button>
			{:else if tab.create}
				<a class="tab-create" href={tab.href}>
					<span class="ring"><Icon name={tab.icon} size={18} /></span>
					<span>{tab.label}</span>
				</a>
			{:else}
				<a
					class="tab-item"
					href={tab.href}
					aria-current={tab.href === section?.href ? 'page' : undefined}
				>
					<span class="ico"><Icon name={tab.icon} size={19} /></span>
					<span>{tab.label}</span>
				</a>
			{/if}
		{/each}
	</nav>

	<SearchPalette bind:open={searching} />
</div>
