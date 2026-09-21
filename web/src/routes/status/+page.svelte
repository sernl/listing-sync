<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import { statusRows } from '$lib/pages/account/status-line';
	import '$lib/pages/account/account.css';

	// The inventories under the shared key, as the Resources board and the
	// resource page cache them; see the registry read below for why one key
	// holds one shape.
	const status = createQuery(() => ({
		queryKey: queryKeys.status,
		queryFn: () => api.status().then((view) => view.inventories)
	}));

	// This page needs no sign-in, so the devices read is allowed to fail: a
	// seller reading it signed out gets the halt states and no device line,
	// which is the honest answer rather than a sign-in wall on the one page
	// that matters when signing in is what is broken. Not retried, because a
	// 401 is an answer and repeating it only delays the rest of the page.
	//
	// The whole view under the shared key, exactly as Settings and
	// Marketplaces cache it. A query key names one shape: this page used to
	// store the array alone, so a visit here after either of those pages read
	// the view found an object where it expected a list and could not be
	// drawn, and a visit there after this one found a list with no
	// `.devices`. The unwrap belongs below, on the read.
	const registry = createQuery(() => ({
		queryKey: queryKeys.devices,
		queryFn: () => api.devices(),
		retry: false
	}));

	const now = Date.now();
	const rows = $derived(
		statusRows(status.data ?? [], registry.data?.devices ?? [], now)
	);
	const paused = $derived(rows.filter((row) => row.tone === 'bad').length);
</script>

<div class="page">
	<PageHead
		icon="activity"
		title="Marketplace status"
		description="Whether each marketplace is working right now."
		guide="connecting"
	>
		{#snippet aside()}
			<!-- Three answers, because a failed read is not an ongoing one. This
			     page matters most when other things are broken, so it must not
			     be the one describing a finished failure as still in flight. -->
			{#if status.isError}
				<StatusPill tone="bad" label="not read" />
			{:else if status.isSuccess}
				<StatusPill
					tone={paused === 0 ? 'ok' : 'warn'}
					label={paused === 0 ? 'all working' : `${paused} paused`}
				/>
			{:else}
				<StatusPill tone="soon" label="checking" />
			{/if}
		{/snippet}
	</PageHead>

	<Panel>
		{#if status.isPending}
			<p class="quiet">Loading…</p>
		{:else if status.isError}
			<p class="quiet">We could not read the status.</p>
		{:else if rows.length === 0}
			<Placeholder
				icon="activity"
				headline="No marketplace to show yet"
				body="Every marketplace we work with appears here."
			/>
		{:else}
			{#each rows as row (row.marketplace)}
				<div class="acct-state-row">
					<span class="who">
						<span class="t"><MarketplaceMark marketplace={row.marketplace} /></span>
						{#if row.why !== ''}
							<span class="why">{row.why}</span>
						{/if}
						{#if row.checked !== ''}
							<span class="why">{row.checked}</span>
						{/if}
					</span>
					<StatusPill tone={row.tone} label={row.label} />
				</div>
			{/each}
		{/if}
	</Panel>
</div>
