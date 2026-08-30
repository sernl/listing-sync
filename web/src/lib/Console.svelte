<script lang="ts">
	import type { Snippet } from 'svelte';
	import { createQuery } from '@tanstack/svelte-query';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { api } from '$lib/api';
	import { present, type StatusPresentation } from '$lib/connection-status';
	import { normaliseQuery } from '$lib/listings-view';
	import {
		NAV_GROUPS,
		SETTINGS_ITEM,
		breadcrumbFor,
		initialsOf,
		isCurrent,
		searchHref
	} from '$lib/nav';
	import { queryKeys } from '$lib/query';

	let { children, onLogout }: { children: Snippet; onLogout: () => void } = $props();

	const organisation = createQuery(() => ({ queryKey: queryKeys.org, queryFn: () => api.org() }));
	const connections = createQuery(() => ({
		queryKey: queryKeys.connections,
		queryFn: () => api.connections()
	}));
	const drain = createQuery(() => ({
		queryKey: queryKeys.drainStats,
		queryFn: () => api.drainStats()
	}));

	const pathname = $derived(page.url.pathname);
	const crumb = $derived(breadcrumbFor(pathname));
	const orgName = $derived(organisation.data?.name);
	const initials = $derived(initialsOf(orgName));
	const links = $derived(connections.data?.connections ?? []);
	const openQuestions = $derived(drain.data?.open ?? null);

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
	// changes and left alone while it is being typed in: on the listings page
	// every keystroke rewrites the URL, and re-reading it here would fight the
	// caret.
	$effect(() => {
		const path = page.url.pathname;
		if (path !== seededFor) {
			seededFor = path;
			query = normaliseQuery(page.url.searchParams.get('q'));
		}
	});

	// On the listings page the box filters as it is typed in, by rewriting the
	// query the table reads; anywhere else it waits for a submit, which is the
	// navigation.
	function typed() {
		if (pathname === '/listings') {
			void goto(searchHref(query), { replaceState: true, keepFocus: true, noScroll: true });
		}
	}

	function submitSearch(event: SubmitEvent) {
		event.preventDefault();
		void goto(searchHref(query), { keepFocus: pathname === '/listings' });
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

<div class="app">
	<aside class="side">
		<div class="wordmark"><span class="leaf" aria-hidden="true">T</span> Teachouse</div>

		{#each NAV_GROUPS as group (group.label)}
			<div class="group-label" id={`nav-${group.label}`}>{group.label}</div>
			<nav aria-labelledby={`nav-${group.label}`}>
				{#each group.items as item (item.href)}
					<a
						class="nav-item"
						href={item.href}
						aria-current={isCurrent(pathname, item.href) ? 'page' : undefined}
					>
						<i class="ico" aria-hidden="true">{item.icon}</i>
						{item.label}
						{#if item.soon}
							<span class="chip">soon</span>
						{:else if item.count === 'reconciliation' && openQuestions !== null && openQuestions > 0}
							<span class="chip count">{openQuestions}</span>
						{/if}
					</a>
				{/each}
			</nav>
		{/each}

		<div class="foot">
			<nav aria-label="Account">
				<a
					class="nav-item"
					href={SETTINGS_ITEM.href}
					aria-current={isCurrent(pathname, SETTINGS_ITEM.href) ? 'page' : undefined}
				>
					<i class="ico" aria-hidden="true">{SETTINGS_ITEM.icon}</i>
					{SETTINGS_ITEM.label}
				</a>
				<button class="nav-item" type="button" onclick={onLogout}>
					<i class="ico" aria-hidden="true">⇥</i>
					Log out
				</button>
			</nav>
			<a class="account" href="/settings">
				<span class="avatar" aria-hidden="true">{initials}</span>
				<span class="who">
					<span class="org">
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
	</aside>

	<main class="main">
		<div class="top">
			<span class="crumb">Console / <b>{crumb}</b></span>
			<span class="grow"></span>
			<span class="conn">
				{#each links as link (link.id)}
					<span title={present(link.status).explanation}>
						<span class="dot {DOT[present(link.status).tone]}"></span>{link.marketplace}
						{link.status}
					</span>
				{/each}
			</span>
			<form class="search" role="search" onsubmit={submitSearch}>
				<span aria-hidden="true">⌕</span>
				<label class="sr-only" for="console-search">Search listings</label>
				<input
					id="console-search"
					name="q"
					type="search"
					placeholder="Search listings…"
					bind:this={searchBox}
					bind:value={query}
					oninput={typed}
				/>
				<kbd>ctrl K</kbd>
			</form>
			<a class="cta" href="/listings">Sync listings</a>
		</div>

		{@render children()}
	</main>
</div>
