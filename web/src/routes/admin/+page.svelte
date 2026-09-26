<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { SIGNUP_DAYS, barWidth, dayLabel, signupPeak, signupSeries } from '$lib/admin';
	import { api } from '$lib/api';
	import { seriesByOrg } from '$lib/drain';
	import Explain from '$lib/Explain.svelte';
	import type { IconName } from '$lib/icons';
	import PageHead from '$lib/PageHead.svelte';
	import StatCard from '$lib/StatCard.svelte';
	import StatusPill, { type Tone } from '$lib/StatusPill.svelte';
	import { queryKeys } from '$lib/query';
	import { DRAIN_WORD, drainVerdict, writeKind } from '$lib/pages/admin/admin-view';
	import { topicLabel } from '$lib/pages/admin/dead-letters';
	import { sumOf, tileFor } from '$lib/pages/admin/ledger-tile';
	import '$lib/flow.css';
	import '$lib/pages/admin/admin.css';

	const signups = createQuery(() => ({
		queryKey: queryKeys.adminSignups,
		queryFn: () => api.adminSignups()
	}));
	const health = createQuery(() => ({
		queryKey: queryKeys.operator,
		queryFn: () => api.adminSyncHealth()
	}));
	const orgs = createQuery(() => ({
		queryKey: queryKeys.adminOrgs,
		queryFn: () => api.adminOrgs().then((view) => view.orgs)
	}));
	const users = createQuery(() => ({
		queryKey: queryKeys.adminUsers,
		queryFn: () => api.adminUsers()
	}));
	const failures = createQuery(() => ({
		queryKey: queryKeys.adminFailures,
		queryFn: () => api.adminFailedWrites().then((view) => view.writes)
	}));
	const drain = createQuery(() => ({
		queryKey: queryKeys.adminImportDrain,
		queryFn: () => api.adminImportDrain()
	}));
	const dead = createQuery(() => ({
		queryKey: queryKeys.adminDeadLetters,
		queryFn: () => api.adminDeadLetters()
	}));

	const rows = $derived(signups.data === undefined ? [] : signupSeries(signups.data, SIGNUP_DAYS));
	const peak = $derived(signupPeak(rows));
	const identityVisible = $derived(signups.data?.identity !== undefined);
	const ledger = $derived(health.data);

	// Undefined rather than zero where a read has not answered, so the glyph,
	// the tint and the figure on each card read one source and cannot come to
	// disagree about whether there is a figure at all.
	const inFlight = $derived(
		sumOf(ledger?.queued, ledger?.leased, ledger?.running, ledger?.verifying)
	);
	const parked = $derived(sumOf(ledger?.parked_live, ledger?.parked_cold));
	const orgCount = $derived(orgs.data?.length);
	const shops = $derived(orgs.data?.reduce((total, org) => total + org.connections, 0));
	/** Organisations on a paid plan, counted once however many members. */
	const paid = $derived(
		users.data === undefined
			? undefined
			: new Set(users.data.users.filter((user) => user.plan !== 'free').map((user) => user.organisation.org)).size
	);
	const writes = $derived(failures.data);
	const stranded = $derived(writes?.filter((write) => writeKind(write) === 'stranded').length);
	const series = $derived(drain.data === undefined ? undefined : seriesByOrg(drain.data.rows).series);
	const notFalling = $derived(
		series?.filter((entry) => drainVerdict(entry.gate) === 'flat') ?? []
	);

	const failedTile = $derived(
		tileFor(writes?.length, { icon: 'circle-x', tone: 'bad' }, { icon: 'circle-check', tone: 'ok' })
	);

	interface Attention {
		key: string;
		tone: Tone;
		word: string;
		what: string;
		count: number;
		href: string;
	}

	/** What an operator should look at, worst first. Only what is non-zero:
	 *  a row reading "0 stranded writes" is a row to skip. */
	const attention = $derived.by(() => {
		const out: Attention[] = [];
		if (stranded !== undefined && stranded > 0) {
			out.push({
				key: 'stranded',
				tone: 'warn',
				word: 'stranded',
				what: 'Writes in flight past their lease',
				count: stranded,
				href: '/admin/failures'
			});
		}
		if (writes !== undefined && writes.length - (stranded ?? 0) > 0) {
			out.push({
				key: 'failed',
				tone: 'bad',
				word: 'failed',
				what: 'Failed writes',
				count: writes.length - (stranded ?? 0),
				href: '/admin/failures'
			});
		}
		for (const topic of dead.data?.topics ?? []) {
			out.push({
				key: `dead-${topic.topic}`,
				tone: 'bad',
				word: 'dead letters',
				what: `${topicLabel(topic.topic)} · ${topic.orgs} ${topic.orgs === 1 ? 'organisation' : 'organisations'}`,
				count: topic.messages,
				href: '/admin/import-drain'
			});
		}
		if (parked !== undefined && parked > 0) {
			out.push({
				key: 'parked',
				tone: 'warn',
				word: 'parked',
				what: 'Sync items parked',
				count: parked,
				href: '/admin/health'
			});
		}
		if (ledger !== undefined && ledger.ambiguous > 0) {
			out.push({
				key: 'ambiguous',
				tone: 'warn',
				word: 'ambiguous',
				what: 'Items settled without knowing if the write landed',
				count: ledger.ambiguous,
				href: '/admin/health'
			});
		}
		if (notFalling.length > 0) {
			out.push({
				key: 'drain',
				tone: DRAIN_WORD.flat.tone,
				word: DRAIN_WORD.flat.label,
				what: `Import share not falling: ${notFalling.map((entry) => entry.org_name).join(', ')}`,
				count: notFalling.length,
				href: '/admin/import-drain'
			});
		}
		return out;
	});

	const loading = $derived(
		health.isPending || failures.isPending || dead.isPending || drain.isPending
	);
	const unread = $derived(
		[health.isError && 'sync health', failures.isError && 'failed writes', dead.isError && 'dead letters', drain.isError && 'import drain'].filter(
			(name): name is string => typeof name === 'string'
		)
	);

	interface Figure {
		href: string;
		icon: IconName;
		tone?: string;
		label: string;
		sub: string;
		value: number | undefined;
	}

	const figures = $derived<Figure[]>([
		{
			href: '/admin/orgs',
			icon: 'building-2',
			label: 'Organisations',
			sub: paid === undefined ? 'accounts' : `${paid} on a paid plan`,
			value: orgCount
		},
		{
			href: '/admin/orgs',
			icon: 'store',
			label: 'Shops connected',
			sub: 'marketplace links held',
			value: shops
		},
		{
			href: '/admin/health',
			icon: 'refresh-cw',
			label: 'Items moving',
			sub: parked === undefined ? 'sync ledger' : `${parked} parked`,
			value: inFlight
		},
		{
			href: '/admin/failures',
			icon: failedTile.icon,
			tone: failedTile.tone,
			label: 'Failed writes',
			sub: stranded === undefined ? 'newest 100' : `${stranded} stranded`,
			value: writes?.length
		},
		{
			href: '/admin/import-drain',
			icon: 'chart-line',
			label: 'Imports draining',
			sub: notFalling.length > 0 ? `${notFalling.length} not falling` : 'organisations measured',
			value: series?.length
		}
	]);
</script>

<div class="page flow-page">
	<PageHead icon="layout-dashboard" title="Overview" description="Every account at a glance. Nothing here changes anything." />

	<div class="flow">
		<div class="op-stats">
			{#each figures as figure (figure.label)}
				<a href={figure.href}>
					<StatCard icon={figure.icon} tone={figure.tone} label={figure.label} sub={figure.sub}>
						{figure.value ?? '—'}
					</StatCard>
				</a>
			{/each}
		</div>

		<section class="flow-section" aria-labelledby="attention-title">
			<div class="flow-section-head">
				<h2 id="attention-title">Needs attention</h2>
			</div>
			{#if unread.length > 0}
				<p class="flow-warn">Not loaded: {unread.join(', ')}.</p>
			{/if}
			{#if loading && attention.length === 0}
				<p class="quiet">Loading…</p>
			{:else if attention.length === 0}
				<p class="quiet">Nothing needs attention.</p>
			{:else}
				<div class="flow-table-wrap op-table op-keep">
					<table class="flow-table">
						<thead>
							<tr><th>Status</th><th>What</th><th class="num">Count</th><th></th></tr>
						</thead>
						<tbody>
							{#each attention as row (row.key)}
								<tr>
									<td data-label="Status"><StatusPill tone={row.tone} label={row.word} /></td>
									<td data-label="What">{row.what}</td>
									<td class="num" data-label="Count">{row.count}</td>
									<td class="num" data-label="Open"><a class="op-open" href={row.href}>Open</a></td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			{/if}
		</section>

		<details class="flow-more">
			<summary>Signups, last {SIGNUP_DAYS} days</summary>
			{#if signups.isPending}
				<p class="quiet">Loading signups…</p>
			{:else if signups.isError}
				<p class="quiet">We could not load signups.</p>
			{:else if rows.length === 0}
				<p class="quiet">Nobody has signed up yet.</p>
			{:else}
				<div class="op-plane-key">
					<span><i class="second" aria-hidden="true"></i> Identity signups</span>
					<span><i aria-hidden="true"></i> Provisioned platform users</span>
					<Explain title="The two signup series" label="">
						<p>
							Identity signups are <span class="mono">user_signed_up</span> events in the identity
							service's audit trail.
						</p>
						<p>
							Provisioned platform users are rows in <span class="mono">app_user</span>, written on
							first sign-in. Someone who registered but never signed in shows in the first series
							only.
						</p>
						<p>Days are UTC days.</p>
					</Explain>
				</div>
				{#each rows as row (row.day)}
					<div class="op-day-row">
						<div class="day">{dayLabel(row.day)}</div>
						<div class="pair">
							<div class="op-plane-bar second">
								{#if row.identity === null}
									<span class="absent">identity trail not visible</span>
								{:else}
									<span class="fill" style={`width:${barWidth(row.identity, peak)}%`}></span>
									<span class="n">{row.identity}</span>
								{/if}
							</div>
							<div class="op-plane-bar">
								<span class="fill" style={`width:${barWidth(row.provisioned, peak)}%`}></span>
								<span class="n">{row.provisioned}</span>
							</div>
						</div>
					</div>
				{/each}
				{#if !identityVisible}
					<p class="op-foot">This database has no identity schema, so the first series is missing, not zero.</p>
				{/if}
			{/if}
		</details>
	</div>
</div>
