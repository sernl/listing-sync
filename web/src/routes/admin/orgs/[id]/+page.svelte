<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { page } from '$app/state';
	import { ApiFailure, api, type GrantRowView } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { present } from '$lib/connection-status';
	import { agoLabel, utcInstant } from '$lib/elapsed';
	import Field from '$lib/Field.svelte';
	import { IMPORT_LADDER, PLANS } from '$lib/generated/plans';
	import type { Plan } from '$lib/generated/vocab';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import StatCard from '$lib/StatCard.svelte';
	import StatusPill, { type Tone } from '$lib/StatusPill.svelte';
	import { toast } from '$lib/toast';
	import { badgeTone } from '$lib/pages/account/tones';
	import { queryKeys } from '$lib/query';
	import '$lib/pages/account/account.css';
	import '$lib/pages/admin/admin.css';

	const orgId = $derived(page.params.id ?? '');

	const detail = createQuery(() => ({
		queryKey: queryKeys.adminOrg(orgId),
		queryFn: () => api.adminOrg(orgId),
		enabled: orgId.length > 0
	}));

	const view = $derived(detail.data);
	const subscription = $derived(view?.subscription);
	const now = Date.now();

	/** Paddle's vocabulary carries no tone of its own, so the states that mean
	 *  money is not arriving are the ones marked. Anything unrecognised is
	 *  rendered plainly rather than guessed at. */
	function tone(status: string): Tone {
		if (status === 'active' || status === 'trialing') {
			return 'ok';
		}
		return status === 'past_due' || status === 'canceled' || status === 'paused' ? 'bad' : 'soon';
	}

	function instant(at: number | undefined | null): string {
		return at === undefined || at === null ? '—' : utcInstant(at);
	}

	const queryClient = useQueryClient();

	/** The plan's own name where the table carries it, and the wire word
	 *  otherwise: a grant written before a plan was renamed still names a
	 *  plan, and printing nothing there would hide which. */
	function planName(plan: Plan): string {
		return PLANS.find((row) => row.id === plan)?.name ?? plan;
	}

	/** Whether a grant is the one in force rather than expired or revoked.
	 *  Only a live grant is worth a Revoke control. */
	function live(grant: GrantRowView): boolean {
		return grant.revoked_at === null && (grant.expires_at === null || grant.expires_at > now);
	}

	let chosen = $state<Plan>('subscriber');
	let rung = $state<number>(IMPORT_LADDER[0]?.up_to ?? 20);
	let expiry = $state('');
	let why = $state('');

	// The rung only means something on the one-off plan: it is the volume the
	// purchase bought, and a rung on a subscription would be a figure nothing
	// reads.
	const needsRung = $derived(chosen === 'migration_only');
	const trimmedWhy = $derived(why.trim());

	/** Why the grant form cannot be submitted, or null where it can. */
	const formRefusal = $derived.by(() => {
		if (trimmedWhy.length === 0) {
			return 'A reason is required: a plan set by hand with no stated why is an audit row that explains nothing.';
		}
		if (expiry !== '' && Number.isNaN(Date.parse(`${expiry}T00:00:00Z`))) {
			return 'That expiry is not a date.';
		}
		return null;
	});

	function refusalOf(failure: Error, fallback: string): string {
		return failure instanceof ApiFailure ? failure.message : fallback;
	}

	async function reload() {
		await queryClient.invalidateQueries({ queryKey: queryKeys.adminOrg(orgId) });
	}

	const granting = createMutation(() => ({
		mutationFn: () =>
			api.grantPlan(orgId, {
				plan: chosen,
				...(needsRung ? { rung } : {}),
				// Midnight UTC on the named day, because the server counts a
				// month in UTC and a local midnight would expire a grant on the
				// wrong day for half the world.
				...(expiry === '' ? {} : { expires_at: Date.parse(`${expiry}T00:00:00Z`) }),
				reason: trimmedWhy
			}),
		onSuccess: async () => {
			await reload();
			why = '';
			expiry = '';
			toast('info', 'Plan set. The tenant sees it on their next request.');
		},
		onError: (failure: Error) => toast('error', refusalOf(failure, 'The plan was not set.'))
	}));

	const revoking = createMutation(() => ({
		mutationFn: (grant: string) => api.revokeGrant(orgId, grant),
		onSuccess: async () => {
			await reload();
			toast('info', 'Grant revoked. The tenant falls back to their next strongest grant.');
		},
		onError: (failure: Error) => toast('error', refusalOf(failure, 'The grant was not revoked.'))
	}));

	function revoke(grant: GrantRowView) {
		const asked = confirm(
			`Revoke the ${planName(grant.plan)} grant on this tenant?\n\n` +
				'They fall back to their next strongest unexpired grant, which for most tenants is nothing at all and therefore Free.'
		);
		if (asked) {
			revoking.mutate(grant.id);
		}
	}
</script>

<div class="page">
	<PageHead
		icon="building-2"
		title={view?.org.name ?? 'Organisation'}
		description={view === undefined ? 'Reading this tenant…' : `Tenant ${view.org.org}`}
	>
		{#snippet aside()}
			<Button tier="outline" icon="arrow-left" href="/admin/orgs">All organisations</Button>
		{/snippet}
	</PageHead>

	{#if detail.isPending}
		<Panel><p class="quiet">Reading this organisation…</p></Panel>
	{:else if detail.isError}
		<Panel><p class="quiet">This organisation could not be read.</p></Panel>
	{:else if view !== undefined}
		<div class="cards">
			<StatCard icon="layout-list" label="Products" sub="not deleted">{view.org.products}</StatCard>
			<StatCard icon="refresh-cw" label="Mappings" sub="listings bound to a marketplace">
				{view.org.mappings}
			</StatCard>
			<StatCard icon="store" label="Connections" sub="marketplace links held">
				{view.org.connections}
			</StatCard>
			<StatCard icon="users" label="Users" sub={`provisioned ${agoLabel(view.org.created_at, now)}`}>
				{view.org.users}
			</StatCard>
		</div>

		<div class="band">
			<Panel
				title="Connections"
				description="The same status the seller's own page renders, derived the same way."
			>
				{#if view.connections.length === 0}
					<Placeholder
						icon="store"
						headline="This tenant has linked no marketplace"
						body="Nothing can sync for them until one is linked, which only they can do."
					/>
				{:else}
					{#each view.connections as link (link.id)}
						<div class="acct-state-row">
							<span class="who">
								<span class="t">{link.marketplace}</span>
								<span class="why">{present(link.status).explanation}</span>
							</span>
							<StatusPill tone={badgeTone(present(link.status).tone)} label={link.status} />
							<StatusPill tone="flat" label={link.state} />
							<span class="why">updated {agoLabel(link.updated_at, now)}</span>
						</div>
					{/each}
				{/if}
			</Panel>

			<Panel title="Billing" description="What Paddle last told us, passed through untranslated.">
				{#if subscription === undefined}
					<Placeholder
						icon="credit-card"
						headline="This tenant has never reached checkout"
						body="That is a different fact from a cancelled subscription, which would appear here carrying Paddle's cancelled status."
					/>
				{:else}
					<dl class="acct-detail">
						<dt>Status</dt>
						<dd><StatusPill tone={tone(subscription.status)} label={subscription.status} /></dd>
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

		<Panel
			title="Plan"
			description="The grant in force, every grant ever written, and the one control on this page that writes."
		>
			<dl class="acct-detail">
				<dt>Plan</dt>
				<dd>{planName(view.plan.plan)}</dd>
				<dt>Rung</dt>
				<dd>{view.plan.rung === null ? '—' : `up to ${view.plan.rung} resources`}</dd>
				<dt>Set by</dt>
				<dd>
					{#if view.plan.granted_by === null}
						<span class="quiet">Nobody — no grant has ever been written, so they are Free</span>
					{:else}
						{view.plan.granted_by}
					{/if}
				</dd>
				<dt>Since</dt>
				<dd>{instant(view.plan.granted_at)}</dd>
				<dt>Expires</dt>
				<dd>
					{#if view.plan.expires_at === null}
						<span class="quiet">Never</span>
					{:else}
						{instant(view.plan.expires_at)}
					{/if}
				</dd>
				<dt>Source</dt>
				<dd>
					{#if view.plan.source_ref === null}
						<span class="quiet">—</span>
					{:else}
						<span class="mono">{view.plan.source_ref}</span>
					{/if}
				</dd>
			</dl>

			<h3>History</h3>
			{#if view.grants.length === 0}
				<p class="quiet">No grant has ever been written for this tenant.</p>
			{:else}
				{#each view.grants as grant (grant.id)}
					<div class="acct-state-row">
						<span class="who">
							<span class="t">
								{planName(grant.plan)}{grant.rung === null ? '' : ` · up to ${grant.rung}`}
							</span>
							<span class="why">
								{grant.granted_by} · {instant(grant.granted_at)}
								{#if grant.reason !== null}· {grant.reason}{/if}
								{#if grant.source_ref !== null}· {grant.source_ref}{/if}
							</span>
						</span>
						{#if grant.revoked_at !== null}
							<StatusPill tone="bad" label="revoked" />
						{:else if grant.expires_at !== null && grant.expires_at <= now}
							<StatusPill tone="soon" label="expired" />
						{:else}
							<StatusPill tone="ok" label="in force" />
						{/if}
						<Button
							small
							danger
							disabled={!live(grant) || revoking.isPending}
							reason={!live(grant)
								? 'This grant is already revoked or expired.'
								: revoking.isPending
									? 'A revoke is in flight.'
									: undefined}
							onclick={() => revoke(grant)}
						>
							Revoke
						</Button>
					</div>
				{/each}
			{/if}

			<h3>Set a plan</h3>
			<p class="foot-note">
				This writes an operator grant against your own operator account. It does not touch
				Paddle, so a tenant who is also paying keeps whichever grant is stronger.
			</p>
			<Field label="Plan" id="grant-plan">
				<select id="grant-plan" bind:value={chosen}>
					{#each PLANS as row (row.id)}
						<option value={row.id}>{row.name}{row.sold ? '' : ' (not sold)'}</option>
					{/each}
				</select>
			</Field>
			{#if needsRung}
				<Field label="Rung" id="grant-rung" hint="The volume the one-off purchase covers.">
					<select id="grant-rung" bind:value={rung}>
						{#each IMPORT_LADDER as step (step.up_to)}
							<option value={step.up_to}>up to {step.up_to} resources</option>
						{/each}
					</select>
				</Field>
			{/if}
			<Field
				label="Expires"
				id="grant-expiry"
				hint="Midnight UTC on this day. Leave empty for a grant that does not lapse."
			>
				<input id="grant-expiry" type="date" bind:value={expiry} />
			</Field>
			<Field label="Reason" id="grant-reason" required>
				<input
					id="grant-reason"
					type="text"
					bind:value={why}
					placeholder="Why this tenant is being given this plan"
				/>
			</Field>
			<div class="actions">
				<Button
					tier="primary"
					disabled={formRefusal !== null || granting.isPending}
					reason={formRefusal ?? (granting.isPending ? 'The grant is being written.' : undefined)}
					onclick={() => granting.mutate()}
				>
					{granting.isPending ? 'Setting…' : 'Set plan'}
				</Button>
			</div>
		</Panel>

		<Panel title="Halts" description="Work this tenant is not allowed to perform right now.">
			{#if view.halts.length === 0}
				<Placeholder
					icon="circle-check"
					headline="Nothing is halted for this tenant"
					body="Every inventory they sell on is accepting work."
				/>
			{:else}
				{#each view.halts as halt (`${halt.inventory ?? 'tenant'}-${halt.raised_at}`)}
					<Banner
						tone="warn"
						title={`${halt.inventory === undefined ? 'The whole tenant' : halt.inventory} — raised ${agoLabel(halt.raised_at, now)} by ${halt.raised_by}`}
					>
						{halt.reason}
					</Banner>
				{/each}
			{/if}
		</Panel>
	{/if}
</div>
