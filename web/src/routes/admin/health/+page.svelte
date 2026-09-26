<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api, type SyncHealthView } from '$lib/api';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import StatCard from '$lib/StatCard.svelte';
	import StatusPill, { type Tone } from '$lib/StatusPill.svelte';
	import { queryKeys } from '$lib/query';
	import { tileFor } from '$lib/pages/admin/ledger-tile';
	import '$lib/pages/admin/admin.css';

	const health = createQuery(() => ({
		queryKey: queryKeys.operator,
		queryFn: () => api.adminSyncHealth()
	}));

	const ledger = $derived(health.data);

	// Three answers rather than two: an unread figure is neither raised nor
	// clear, and `count` below already prints an em dash for it.
	const settledTile = $derived(
		tileFor(ledger?.settled, { icon: 'circle-check', tone: 'ok' }, { icon: 'minus', tone: 'ok' })
	);
	const failedTile = $derived(
		tileFor(ledger?.failed, { icon: 'circle-x', tone: 'bad' }, { icon: 'circle-check', tone: 'ok' })
	);

	/** The stored state vocabulary, uncollapsed. A seller's job page folds
	 *  leased, running and verifying into one figure and both park states into
	 *  another; that is the right rendering there and the wrong one here, since
	 *  parked-live against parked-cold is the distinction an operator opened
	 *  this page to find. */
	const STATES: readonly { key: keyof SyncHealthView; label: string; note: string }[] = [
		{ key: 'queued', label: 'Queued', note: 'waiting for a worker to take them' },
		{ key: 'leased', label: 'Leased', note: 'claimed by a worker, not yet started' },
		{ key: 'running', label: 'Running', note: 'a write is in progress' },
		{ key: 'verifying', label: 'Verifying', note: 'written, reading back to confirm' },
		{ key: 'blocked', label: 'Blocked', note: 'waiting on something else to settle first' },
		{ key: 'parked_live', label: 'Parked (live)', note: 'held with the connection still usable' },
		{ key: 'parked_cold', label: 'Parked (cold)', note: 'held with nothing usable stored' },
		{ key: 'settled', label: 'Settled', note: 'finished, with an outcome below' }
	];

	const OUTCOMES: readonly { key: keyof SyncHealthView; label: string; tone: Tone }[] = [
		{ key: 'succeeded', label: 'Succeeded', tone: 'ok' },
		{ key: 'degraded', label: 'Degraded', tone: 'run' },
		{ key: 'failed', label: 'Failed', tone: 'bad' },
		{ key: 'ambiguous', label: 'Ambiguous', tone: 'bad' },
		{ key: 'skipped', label: 'Skipped', tone: 'soon' },
		{ key: 'outcome_blocked', label: 'Blocked', tone: 'soon' }
	];

	/** One figure from the ledger, or an em dash.
	 *
	 *  Guarded on the figure rather than on the ledger: a body that arrived
	 *  without a field leaves the ledger defined and the field undefined, and
	 *  guarding only the ledger printed the string "undefined" into the card. */
	function count(key: keyof SyncHealthView): string {
		const figure = ledger?.[key];
		return figure === undefined ? '—' : String(figure);
	}
</script>

<div class="page">
	<PageHead
		icon="heart-pulse"
		title="Sync health"
		description="Every item across all accounts, by the state the database stores."
	/>

	{#if health.isPending}
		<Panel><p class="quiet">Loading sync health…</p></Panel>
	{:else if health.isError}
		<Panel><p class="quiet">We could not load sync health.</p></Panel>
	{:else}
		<div class="cards">
			<StatCard icon="refresh-cw" label="Sync runs" sub="all accounts">{count('jobs')}</StatCard>
			<StatCard icon="layout-list" label="Items" sub="all runs, all accounts">
				{count('items')}
			</StatCard>
			<StatCard
				icon={settledTile.icon}
				tone={settledTile.tone}
				label="Settled"
				sub="finished, with an outcome"
			>
				{count('settled')}
			</StatCard>
			<StatCard
				icon={failedTile.icon}
				tone={failedTile.tone}
				label="Failed"
				sub="settled with a failure"
			>
				{count('failed')}
			</StatCard>
		</div>

		<div class="band">
			<Panel title="Items by state" description="Where every item in the ledger currently sits.">
				<div class="op-table">
					<table>
						<thead>
							<tr><th>State</th><th>What it means</th><th class="num">Items</th></tr>
						</thead>
						<tbody>
							{#each STATES as state (state.key)}
								<tr>
									<td class="op-cell" data-label="State">
										<span class="t" title={state.label}>{state.label}</span>
									</td>
									<td data-label="What it means"><span class="s">{state.note}</span></td>
									<td class="num" data-label="Items">{count(state.key)}</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			</Panel>

			<Panel title="Settled outcomes" description="How the finished items finished.">
				{#each OUTCOMES as outcome (outcome.key)}
					<div class="op-tally">
						<StatusPill tone={outcome.tone} label={outcome.label} />
						<span class="n">{count(outcome.key)}</span>
					</div>
				{/each}
				<p class="foot-note">
					These add up to the Settled figure above. There is no single health score, because
					an average would hide the number that matters.
				</p>
			</Panel>
		</div>
	{/if}
</div>
