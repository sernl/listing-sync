<script lang="ts">
	import { createMutation } from '@tanstack/svelte-query';
	import { ApiFailure, api } from '$lib/api';
	import Button from '$lib/Button.svelte';
	import Field from '$lib/Field.svelte';
	import { PLANS } from '$lib/generated/plans';
	import type { Plan } from '$lib/generated/vocab';
	import { toast } from '$lib/toast';

	// The two controls on the operator surface that write an entitlement: the
	// plan a tenant holds, and the moves it can spend. A component rather than
	// markup inside the organisation page because the users page sets a plan
	// on the same organisation from a dialog, and two copies of this form are
	// two forms to keep in step with what the server requires.
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
	let expiry = $state('');
	let why = $state('');

	const trimmedWhy = $derived(why.trim());

	/** Why the grant form cannot be submitted, or null where it can. */
	const formRefusal = $derived.by(() => {
		if (trimmedWhy.length === 0) {
			return 'Enter a reason. It goes in the audit log.';
		}
		if (expiry !== '' && Number.isNaN(Date.parse(`${expiry}T00:00:00Z`))) {
			return 'Enter a valid expiry date.';
		}
		return null;
	});

	const granting = createMutation(() => ({
		mutationFn: () =>
			api.grantPlan(org, {
				plan: chosen,
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
			toast('info', 'Plan set. The account sees it on its next request.');
		},
		onError: (failure: Error) =>
			toast('error', failure instanceof ApiFailure ? failure.message : 'The plan was not set.')
	}));

	// The moves half. Signed, because an operator reaches for this to correct
	// a balance as often as to give moves away, and a correction that could
	// only add would leave a double credit standing.
	let delta = $state(0);
	let creditWhy = $state('');

	const trimmedCreditWhy = $derived(creditWhy.trim());

	/** Why the credit cannot be submitted, or null where it can. */
	const creditRefusal = $derived.by(() => {
		if (!Number.isInteger(delta) || delta === 0) {
			return 'Enter a number of moves other than zero.';
		}
		if (trimmedCreditWhy.length === 0) {
			return 'Enter a reason. The same reason twice counts as one credit.';
		}
		return null;
	});

	const crediting = createMutation(() => ({
		mutationFn: () => api.creditMoves(org, { moves: delta, reason: trimmedCreditWhy }),
		onSuccess: async () => {
			await onGranted();
			delta = 0;
			creditWhy = '';
			toast('info', 'Balance set. The account sees it on its next request.');
		},
		onError: (failure: Error) =>
			toast(
				'error',
				failure instanceof ApiFailure ? failure.message : 'The balance was not changed.'
			)
	}));
</script>

<div class="op-grant">
	<p class="foot-note">
		This sets an operator grant under your operator account. It does not touch Stripe, so an
		account that also pays keeps whichever grant is stronger.
	</p>
	<Field label="Plan" id={`grant-plan-${org}`}>
		<select id={`grant-plan-${org}`} bind:value={chosen}>
			{#each PLANS as row (row.id)}
				<option value={row.id}>{row.name}{row.sold ? '' : ' (not sold)'}</option>
			{/each}
		</select>
	</Field>
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
			placeholder="Why this account gets this plan"
		/>
	</Field>
	<div class="actions">
		<Button
			tier="primary"
			disabled={formRefusal !== null || granting.isPending}
			reason={formRefusal ?? (granting.isPending ? 'Setting the plan.' : undefined)}
			onclick={() => granting.mutate()}
		>
			{granting.isPending ? 'Setting…' : 'Set plan'}
		</Button>
	</div>

	<hr />

	<p class="foot-note">Give this account moves to spend, or take back moves credited twice.</p>
	<Field label="Moves" id={`credit-moves-${org}`} hint="Negative takes moves back.">
		<input id={`credit-moves-${org}`} type="number" step="1" bind:value={delta} />
	</Field>
	<Field label="Reason" id={`credit-reason-${org}`} required>
		<input
			id={`credit-reason-${org}`}
			type="text"
			bind:value={creditWhy}
			placeholder="Why this balance is being changed"
		/>
	</Field>
	<div class="actions">
		<Button
			disabled={creditRefusal !== null || crediting.isPending}
			reason={creditRefusal ?? (crediting.isPending ? 'Crediting moves.' : undefined)}
			onclick={() => crediting.mutate()}
		>
			{crediting.isPending ? 'Crediting…' : 'Credit moves'}
		</Button>
	</div>
</div>
