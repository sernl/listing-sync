<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api } from '$lib/api';
	import { agoLabel, utcInstant } from '$lib/elapsed';
	import Explain from '$lib/Explain.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import { MARKETPLACE_NAME, SHORT_NAME } from '$lib/platforms';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import { statusRows } from '$lib/pages/account/status-line';
	import { STATE_MEANINGS, statusAdvice, statusEvents } from '$lib/pages/marketplaces/status';
	import '$lib/flow.css';
	import '$lib/pages/marketplaces/marketplaces.css';

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
	const events = $derived(statusEvents(status.data ?? [], registry.data?.devices ?? []));
</script>

<div class="page flow-page">
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
				<StatusPill tone="bad" label="could not check" />
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

	{#if status.isPending}
		<p class="quiet">Loading…</p>
	{:else if status.isError}
		<p class="quiet">We could not check your marketplaces. Reload the page to try again.</p>
	{:else if rows.length === 0}
		<Placeholder
			icon="activity"
			headline="No marketplace to show yet"
			body="Each marketplace we work with shows here."
		/>
	{:else}
		<div class="flow">
			<section class="flow-section">
				<div class="flow-section-head">
					<h2>Your marketplaces</h2>
					<Explain title="What each state means" label="What the states mean">
						{#each STATE_MEANINGS as state (state.label)}
							<p><StatusPill tone={state.tone} label={state.label} /> {state.meaning}</p>
						{/each}
					</Explain>
				</div>
				<div class="st-cards">
					{#each rows as row (row.marketplace)}
						<article class="st-card" class:st-bad={row.tone === 'bad'}>
							<div class="st-head">
								<MarketplaceMark marketplace={row.marketplace} size={32} />
								<span class="st-name" title={row.name}>{SHORT_NAME[row.marketplace]}</span>
								<StatusPill tone={row.tone} label={row.label} />
							</div>
							{#if row.why !== '' && row.tone === 'bad'}
								<p class="st-why">{row.why}</p>
							{/if}
							{#if row.label !== 'Coming soon'}
								<p class="st-line">
									<span class="st-key">Last check</span>
									{row.checked === '' ? 'Not yet' : row.checked}
								</p>
							{/if}
							<p class="st-line">
								<span class="st-key">What to do</span>
								{statusAdvice(row)}
							</p>
						</article>
					{/each}
				</div>
			</section>

			<section class="flow-section">
				<div class="flow-section-head"><h2>Recent events</h2></div>
				{#if events.length === 0}
					<p class="quiet">Nothing has happened here yet.</p>
				{:else}
					<div class="flow-table-wrap">
						<table class="flow-table">
							<thead>
								<tr><th>When</th><th>Shop</th><th>What happened</th></tr>
							</thead>
							<tbody>
								{#each events as event, index (index)}
									<tr>
										<td class="st-when" title={utcInstant(event.at)}>{agoLabel(event.at, now)}</td>
										<td class="marks">
											<MarketplaceMark marketplace={event.marketplace} />
											<span class="sr-only">{MARKETPLACE_NAME[event.marketplace]}</span>
										</td>
										<td class:st-bad-ink={event.kind === 'paused'}>{event.what}</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</div>
				{/if}
			</section>
		</div>
	{/if}
</div>

<style>
	.st-cards {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
		gap: var(--s-4);
	}

	.st-card {
		display: flex;
		flex-direction: column;
		gap: var(--s-2);
		padding: var(--s-4);
		border: 1px solid var(--line);
		border-radius: var(--r-panel);
		background: var(--card);
		box-shadow: var(--sh-1);
		min-width: 0;
	}

	.st-card.st-bad {
		border-color: color-mix(in srgb, var(--bad) 40%, var(--line));
	}

	.st-head {
		display: flex;
		align-items: center;
		gap: var(--s-3);
		margin-bottom: var(--s-2);
	}

	.st-name {
		flex: 1 1 auto;
		min-width: 0;
		font-family: var(--display);
		font-size: 17px;
		font-weight: 600;
	}

	.st-why {
		margin: 0;
		color: var(--bad-ink);
		font-size: 13px;
	}

	.st-line {
		display: flex;
		flex-direction: column;
		margin: 0;
		font-size: 13.5px;
	}

	.st-key {
		color: var(--muted);
		font-size: 11.5px;
		font-weight: 600;
		letter-spacing: 0.04em;
		text-transform: uppercase;
	}

	.st-when {
		white-space: nowrap;
		color: var(--muted);
		font-variant-numeric: tabular-nums;
	}

	.st-bad-ink {
		color: var(--bad-ink);
	}
</style>
