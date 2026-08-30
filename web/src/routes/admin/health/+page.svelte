<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { api, type SyncHealthView } from '$lib/api';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import StatCard from '$lib/StatCard.svelte';
	import { queryKeys } from '$lib/query';

	const health = createQuery(() => ({
		queryKey: queryKeys.operator,
		queryFn: () => api.adminSyncHealth()
	}));

	const ledger = $derived(health.data);

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

	const OUTCOMES: readonly { key: keyof SyncHealthView; label: string; tone: string }[] = [
		{ key: 'succeeded', label: 'Succeeded', tone: 'ok' },
		{ key: 'degraded', label: 'Degraded', tone: 'run' },
		{ key: 'failed', label: 'Failed', tone: 'bad' },
		{ key: 'ambiguous', label: 'Ambiguous', tone: 'bad' },
		{ key: 'skipped', label: 'Skipped', tone: 'mut' },
		{ key: 'outcome_blocked', label: 'Blocked', tone: 'mut' }
	];

	function count(key: keyof SyncHealthView): string {
		return ledger === undefined ? '—' : String(ledger[key]);
	}
</script>

<div class="page">
	<PageHead
		icon="❤"
		title="Sync health"
		description="The ledger across every tenant, in the states the database actually stores."
	/>

	{#if health.isPending}
		<Panel><p class="quiet">Reading the ledger…</p></Panel>
	{:else if health.isError}
		<Panel><p class="quiet">The ledger could not be read.</p></Panel>
	{:else}
		<div class="cards">
			<StatCard icon="⇄" label="Sync runs" sub="every tenant">{count('jobs')}</StatCard>
			<StatCard icon="▤" label="Items" sub="every run, every tenant">{count('items')}</StatCard>
			<StatCard
				icon={(ledger?.settled ?? 0) > 0 ? '✓' : '·'}
				tone="ok"
				label="Settled"
				sub="finished, with an outcome"
			>
				{count('settled')}
			</StatCard>
			<StatCard
				icon={(ledger?.failed ?? 0) > 0 ? '✕' : '✓'}
				tone={(ledger?.failed ?? 0) > 0 ? 'bad' : 'ok'}
				label="Failed"
				sub="settled with a failure"
			>
				{count('failed')}
			</StatCard>
		</div>

		<div class="band">
			<Panel title="Items by state" description="Where every item in the ledger currently sits.">
				<div class="tbl-wrap">
					<table>
						<thead>
							<tr><th>State</th><th>What it means</th><th class="num">Items</th></tr>
						</thead>
						<tbody>
							{#each STATES as state (state.key)}
								<tr>
									<td class="title-cell"><div class="t">{state.label}</div></td>
									<td class="s">{state.note}</td>
									<td class="num">{count(state.key)}</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			</Panel>

			<Panel title="Settled outcomes" description="How the finished items finished.">
				{#each OUTCOMES as outcome (outcome.key)}
					<div class="row">
						<span class="pill {outcome.tone}">{outcome.label}</span>
						<span class="grow"></span>
						<span class="t">{count(outcome.key)}</span>
					</div>
				{/each}
				<p class="foot-note">
					These sum to the settled figure above. Nothing here is collapsed into a single health
					score: a number that averaged these would hide the one that matters.
				</p>
			</Panel>
		</div>
	{/if}
</div>
