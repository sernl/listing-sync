<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { SIGNUP_DAYS, barWidth, dayLabel, signupPeak, signupSeries } from '$lib/admin';
	import { api } from '$lib/api';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import StatCard from '$lib/StatCard.svelte';
	import { queryKeys } from '$lib/query';

	const signups = createQuery(() => ({
		queryKey: queryKeys.adminSignups,
		queryFn: () => api.adminSignups()
	}));
	const health = createQuery(() => ({
		queryKey: queryKeys.operator,
		queryFn: () => api.adminSyncHealth()
	}));

	const rows = $derived(signups.data === undefined ? [] : signupSeries(signups.data, SIGNUP_DAYS));
	const peak = $derived(signupPeak(rows));
	const identityVisible = $derived(signups.data?.identity !== undefined);
	const ledger = $derived(health.data);
	const inFlight = $derived(
		ledger === undefined ? 0 : ledger.queued + ledger.leased + ledger.running + ledger.verifying
	);
	const parked = $derived(ledger === undefined ? 0 : ledger.parked_live + ledger.parked_cold);
</script>

<div class="page">
	<PageHead
		icon="◈"
		title="Platform overview"
		description="Every tenant at once. Read-only: nothing on this page writes anything."
	/>

	<div class="cards">
		<StatCard icon="⇄" label="Sync runs" sub="across every tenant">
			{ledger?.jobs ?? '—'}
		</StatCard>
		<StatCard icon="▤" tag="in flight" label="Items moving" sub="queued, leased, running or verifying">
			{ledger === undefined ? '—' : inFlight}
		</StatCard>
		<StatCard
			icon={parked > 0 ? '⏸' : '✓'}
			tone={parked > 0 ? 'warn' : 'ok'}
			label="Parked items"
			sub={ledger === undefined
				? 'the ledger has not been read'
				: `${ledger.parked_live} live, ${ledger.parked_cold} cold`}
		>
			{ledger === undefined ? '—' : parked}
		</StatCard>
		<StatCard
			icon={(ledger?.failed ?? 0) > 0 ? '✕' : '✓'}
			tone={(ledger?.failed ?? 0) > 0 ? 'bad' : 'ok'}
			label="Failed items"
			sub={ledger === undefined ? 'the ledger has not been read' : `${ledger.settled} settled in all`}
		>
			{ledger?.failed ?? '—'}
		</StatCard>
	</div>

	<Panel
		title="Signups over time"
		description="The newest {SIGNUP_DAYS} days each plane recorded, by UTC day."
	>
		{#snippet more()}
			<a class="more" href="/admin/health">Sync health</a>
		{/snippet}

		{#if signups.isPending}
			<p class="quiet">Reading the signup series…</p>
		{:else if signups.isError}
			<p class="quiet">The signup series could not be read.</p>
		{:else if rows.length === 0}
			<div class="clear">
				<span class="big" aria-hidden="true">◈</span>
				Nobody has signed up yet on either plane.
			</div>
		{:else}
			<div class="series-key">
				<span><i class="swatch second" aria-hidden="true"></i> Identity signups</span>
				<span><i class="swatch" aria-hidden="true"></i> Provisioned platform users</span>
			</div>

			{#each rows as row (row.day)}
				<div class="bar-row">
					<div class="day">{dayLabel(row.day)}</div>
					<div class="bar-pair">
						<div class="bar second">
							{#if row.identity === null}
								<span class="absent">identity trail not visible from this database</span>
							{:else}
								<span class="fill" style={`width:${barWidth(row.identity, peak)}%`}></span>
								<span class="n">{row.identity}</span>
							{/if}
						</div>
						<div class="bar">
							<span class="fill" style={`width:${barWidth(row.provisioned, peak)}%`}></span>
							<span class="n">{row.provisioned}</span>
						</div>
					</div>
				</div>
			{/each}

			<p class="foot-note">
				Identity signups are <span class="mono">user_signed_up</span> events in the identity
				service's audit trail. Provisioned platform users are rows in
				<span class="mono">app_user</span>, which the session exchange writes the first time a
				subject signs in — so a human who registered and never came back appears in the first
				series and not the second.
				{#if !identityVisible}
					This database carries no identity schema, so the first series is absent rather than
					zero.
				{/if}
			</p>
		{/if}
	</Panel>
</div>
