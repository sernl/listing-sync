<script lang="ts">
	import type { Snippet } from 'svelte';
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { page } from '$app/state';
	import { goto, invalidateAll } from '$app/navigation';
	import { impersonationState, operatorVerdict } from '$lib/admin';
	import { api } from '$lib/api';
	import { impersonatedSession, stopImpersonatingAndRestore } from '$lib/auth-client';
	import { present, type StatusPresentation } from '$lib/connection-status';
	import Icon from '$lib/Icon.svelte';
	import ImpersonationBanner from '$lib/ImpersonationBanner.svelte';
	import { normaliseQuery } from '$lib/listings-view';
	import {
		ADMIN_SECTION,
		CREATE_TAB,
		HOME_ITEM,
		PHONE_BAR,
		SECTIONS,
		breadcrumbFor,
		currentDestination,
		initialsOf,
		searchHref,
		sectionFor
	} from '$lib/nav';
	import { queryKeys } from '$lib/query';

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

	const pathname = $derived(page.url.pathname);
	const crumb = $derived(breadcrumbFor(pathname));
	const orgName = $derived(organisation.data?.name);
	const initials = $derived(initialsOf(orgName));
	const links = $derived(connections.data?.connections ?? []);

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

	let query = $state('');
	let searchBox = $state<HTMLInputElement | null>(null);
	let seededFor = '';

	// The URL carries the filter, so the box is seeded from it when the page
	// changes and left alone while it is being typed in: on the inventory board
	// every keystroke rewrites the URL, and re-reading it here would fight the
	// caret.
	$effect(() => {
		const path = page.url.pathname;
		if (path !== seededFor) {
			seededFor = path;
			query = normaliseQuery(page.url.searchParams.get('q'));
		}
	});

	// On the inventory board the box filters as it is typed in, by rewriting
	// the query the table reads; anywhere else it waits for a submit, which is
	// the navigation.
	function typed() {
		if (pathname === '/inventory') {
			void goto(searchHref(query), { replaceState: true, keepFocus: true, noScroll: true });
		}
	}

	function submitSearch(event: SubmitEvent) {
		event.preventDefault();
		void goto(searchHref(query), { keepFocus: pathname === '/inventory' });
	}

	function shortcut(event: KeyboardEvent) {
		if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {
			event.preventDefault();
			searchBox?.focus();
			searchBox?.select();
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
			<form class="search" role="search" onsubmit={submitSearch}>
				<Icon name="search" size={15} />
				<label class="sr-only" for="console-search">Search resources</label>
				<input
					id="console-search"
					name="q"
					type="search"
					placeholder="Search resources…"
					bind:this={searchBox}
					bind:value={query}
					oninput={typed}
				/>
				<kbd>ctrl K</kbd>
			</form>
			<a class="cta" href={CREATE_TAB.href}>{CREATE_TAB.label}</a>
			<a class="account" href="/settings">
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
					<span class="plan">{page.url.host}</span>
				</span>
			</a>
		</div>

		{@render children()}
	</main>

	<nav class="tabbar" aria-label="Sections">
		{#each PHONE_BAR as tab (tab.href)}
			{#if tab.create}
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
</div>
