<script lang="ts">
	import { createMutation } from '@tanstack/svelte-query';
	import { ApiFailure, api } from '$lib/api';
	import Button from '$lib/Button.svelte';
	import Field from '$lib/Field.svelte';
	import { IMPORT_LADDER, PLANS } from '$lib/generated/plans';
	import type { Plan } from '$lib/generated/vocab';
	import { toast } from '$lib/toast';

	// The one control on the operator surface that writes an entitlement. A
	// component rather than markup inside the organisation page because the
	// users page sets a plan on the same organisation from a dialog, and two
	// copies of this form are two forms to keep in step with what the server
	// requires of a grant.
	let {
		org,
		onGranted
	}: {
		org: string;
		/** Re-read whatever the host draws from. The server answers the whole
		 *  org detail back, but each host renders a different part of it, so
		 *  the host invalidates rather than this taking the answer apart. */
		onGranted: () => Promise<void> | void;
	} = $props();

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

	const granting = createMutation(() => ({
		mutationFn: () =>
			api.grantPlan(org, {
				plan: chosen,
				...(needsRung ? { rung } : {}),
				// Midnight UTC on the named day, because the server counts a
				// month in UTC and a local midnight would expire a grant on the
				// wrong day for half the world.
				...(expiry === '' ? {} : { expires_at: Date.parse(`${expiry}T00:00:00Z`) }),
				reason: trimmedWhy
			}),
		onSuccess: async () => {
			await onGranted();
			why = '';
			expiry = '';
			toast('info', 'Plan set. The tenant sees it on their next request.');
		},
		onError: (failure: Error) =>
			toast(
				'error',
				failure instanceof ApiFailure ? failure.message : 'The plan was not set.'
			)
	}));
</script>

<div class="op-grant">
	<p class="foot-note">
		This writes an operator grant against your own operator account. It does not touch Paddle,
		so a tenant who is also paying keeps whichever grant is stronger.
	</p>
	<Field label="Plan" id={`grant-plan-${org}`}>
		<select id={`grant-plan-${org}`} bind:value={chosen}>
			{#each PLANS as row (row.id)}
				<option value={row.id}>{row.name}{row.sold ? '' : ' (not sold)'}</option>
			{/each}
		</select>
	</Field>
	{#if needsRung}
		<Field label="Rung" id={`grant-rung-${org}`} hint="The volume the one-off purchase covers.">
			<select id={`grant-rung-${org}`} bind:value={rung}>
				{#each IMPORT_LADDER as step (step.up_to)}
					<option value={step.up_to}>up to {step.up_to} resources</option>
				{/each}
			</select>
		</Field>
	{/if}
	<Field
		label="Expires"
		id={`grant-expiry-${org}`}
		hint="Midnight UTC on this day. Leave empty for a grant that does not lapse."
	>
		<input id={`grant-expiry-${org}`} type="date" bind:value={expiry} />
	</Field>
	<Field label="Reason" id={`grant-reason-${org}`} required>
		<input
			id={`grant-reason-${org}`}
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
</div>
