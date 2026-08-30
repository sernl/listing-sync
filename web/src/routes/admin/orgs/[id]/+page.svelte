<script lang="ts">
	import { createQuery } from '@tanstack/svelte-query';
	import { page } from '$app/state';
	import { api } from '$lib/api';
	import { present } from '$lib/connection-status';
	import { agoLabel } from '$lib/elapsed';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import StatCard from '$lib/StatCard.svelte';
	import { queryKeys } from '$lib/query';

	const orgId = $derived(page.params.id ?? '');

	const detail = createQuery(() => ({
		queryKey: queryKeys.adminOrg(orgId),
		queryFn: () => api.adminOrg(orgId),
		enabled: orgId.length > 0
	}));

	const view = $derived(detail.data);
	const subscription = $derived(view?.subscription);
	const now = Date.now();

	/** Paddle's vocabulary carries no tone of its own, so the two states that
	 *  mean money is not arriving are the ones marked. Anything unrecognised is
	 *  rendered plainly rather than guessed at. */
	function tone(status: string): string {
		if (status === 'active' || status === 'trialing') {
			return 'ok';
		}
		return status === 'past_due' || status === 'canceled' || status === 'paused' ? 'bad' : 'mut';
	}

	function instant(at: number | undefined): string {
		return at === undefined ? '—' : new Date(at).toISOString().replace('T', ' ').slice(0, 16);
	}
</script>

<div class="page">
	<PageHead
		icon="⌂"
		title={view?.org.name ?? 'Organisation'}
		description={view === undefined ? 'Reading this tenant…' : `Tenant ${view.org.org}`}
	>
		{#snippet aside()}
			<a class="btn" href="/admin/orgs">All organisations</a>
		{/snippet}
	</PageHead>

	{#if detail.isPending}
		<Panel><p class="quiet">Reading this organisation…</p></Panel>
	{:else if detail.isError}
		<Panel><p class="quiet">This organisation could not be read.</p></Panel>
	{:else if view !== undefined}
		<div class="cards">
			<StatCard icon="▤" label="Products" sub="not deleted">{view.org.products}</StatCard>
			<StatCard icon="⇄" label="Mappings" sub="listings bound to a marketplace">
				{view.org.mappings}
			</StatCard>
			<StatCard icon="⚲" label="Connections" sub="marketplace links held">
				{view.org.connections}
			</StatCard>
			<StatCard icon="☺" label="Users" sub={`provisioned ${agoLabel(view.org.created_at, now)}`}>
				{view.org.users}
			</StatCard>
		</div>

		<div class="band">
			<Panel
				title="Connections"
				description="The same status the seller's own page renders, derived the same way."
			>
				{#if view.connections.length === 0}
					<div class="clear">
						<span class="big" aria-hidden="true">⚲</span>
						This tenant has linked no marketplace.
					</div>
				{:else}
					{#each view.connections as link (link.id)}
						<div class="row">
							<span class="pill {present(link.status).tone}">{link.status}</span>
							<span class="what">
								<span class="t">{link.marketplace}</span>
								<span class="s">{present(link.status).explanation}</span>
							</span>
							<span class="grow"></span>
							<span class="badge">{link.state}</span>
							<span class="s">{agoLabel(link.updated_at, now)}</span>
						</div>
					{/each}
				{/if}
			</Panel>

			<Panel title="Billing" description="What Paddle last told us, passed through untranslated.">
				{#if subscription === undefined}
					<div class="clear">
						<span class="big" aria-hidden="true">◇</span>
						This tenant has never reached checkout. That is a different fact from a cancelled
						subscription, which would appear here carrying Paddle's cancelled status.
					</div>
				{:else}
					<dl class="facts">
						<dt>Status</dt>
						<dd><span class="pill {tone(subscription.status)}">{subscription.status}</span></dd>
						<dt>Current period ends</dt>
						<dd>
							{#if subscription.current_period_end === undefined}
								<span class="quiet">Paddle carried no billing period</span>
							{:else}
								{instant(subscription.current_period_end)}
							{/if}
						</dd>
						<dt>As of</dt>
						<dd>{instant(subscription.occurred_at)}</dd>
					</dl>
					<p class="foot-note">
						“As of” is the instant Paddle stamped on the notification that produced this state,
						not the instant we recorded it.
					</p>
				{/if}
			</Panel>
		</div>

		<Panel title="Halts" description="Work this tenant is not allowed to perform right now.">
			{#if view.halts.length === 0}
				<div class="clear">
					<span class="big" aria-hidden="true">✓</span>
					Nothing is halted for this tenant.
				</div>
			{:else}
				{#each view.halts as halt (`${halt.inventory ?? 'tenant'}-${halt.raised_at}`)}
					<div class="attn">
						<div class="t">
							{halt.inventory === undefined ? 'The whole tenant' : halt.inventory} — raised by {halt.raised_by}
						</div>
						<p>{halt.reason}</p>
						<span class="s">{agoLabel(halt.raised_at, now)}</span>
					</div>
				{/each}
			{/if}
		</Panel>
	{/if}
</div>
