<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { page } from '$app/state';
	import { ApiFailure, api, type OrgView } from '$lib/api';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import Field from '$lib/Field.svelte';
	import Icon from '$lib/Icon.svelte';
	import { nameFromSlug } from '$lib/org-slug';
	import { queryKeys } from '$lib/query';
	import { NO_PROBE, claimBlockedReason, claimStatus, worthProbing, type Probe } from './state';
	import './claim.css';

	let {
		/** The banner variant is the same screen shown to an organisation that
		 *  predates the slug, which reaches it by choice rather than by a gate,
		 *  so it is the only one offered a way out. */
		dismissable = false,
		onDone
	}: { dismissable?: boolean; onDone: () => void } = $props();

	const queryClient = useQueryClient();

	/** How long the field rests before a probe is spent on it. Long enough that
	 *  typing a name costs one probe rather than one per keystroke, short enough
	 *  that the answer arrives while the seller is still looking at the field. */
	const SETTLE_MS = 350;

	let slugDraft = $state('');
	let nameDraft = $state('');
	let nameEdited = $state(false);
	let refusal = $state<string | null>(null);
	let settled = $state('');

	// The proposal stops the moment the seller edits the name themselves, so a
	// later keystroke in the slug field can never overwrite what they typed.
	$effect(() => {
		const proposed = worthProbing(slugDraft);
		if (!nameEdited) {
			nameDraft = proposed === null ? '' : nameFromSlug(proposed);
		}
	});

	$effect(() => {
		const candidate = worthProbing(slugDraft);
		if (candidate === null) {
			settled = '';
			return;
		}
		const timer = setTimeout(() => {
			settled = candidate;
		}, SETTLE_MS);
		return () => clearTimeout(timer);
	});

	const availability = createQuery(() => ({
		queryKey: queryKeys.orgSlug(settled),
		queryFn: () => api.slugAvailability(settled),
		enabled: settled.length > 0,
		// Never retried. The route is bounded per session, so a retry spends
		// budget the seller needs for the name they are actually typing, and the
		// verdict is advisory in any case.
		retry: false
	}));

	// The probe names the slug it is about whether it answered or not. Deriving
	// that name from `data` alone would leave a failed probe naming nothing, and
	// a probe naming nothing reads as a probe for some other value -- so a check
	// that failed would render as one still running, for ever.
	const probe = $derived<Probe>(
		settled.length === 0
			? NO_PROBE
			: {
					slug: availability.data?.slug ?? settled,
					pending: availability.isPending,
					available: availability.data?.available ?? null,
					failed: availability.isError
				}
	);

	const status = $derived(claimStatus(slugDraft, probe));

	const claiming = createMutation(() => ({
		mutationFn: (change: { name?: string; slug: string }) => api.updateOrg(change),
		onSuccess: (stored: OrgView) => {
			// The cache takes the stored row rather than what was submitted: the
			// server normalises the slug and answers with what it kept. Reloading
			// the session -- which is what lifts the gate -- is the caller's, so
			// this component knows nothing about routing.
			queryClient.setQueryData(queryKeys.org, stored);
			onDone();
		},
		onError: (failure: Error) => {
			// A 409 says someone else holds the name and a 422 says the name is
			// malformed or reserved. Both are about the field, so both are
			// answered beside it -- including the 409 the availability check said
			// would not happen, because only the server's index decides.
			refusal =
				failure instanceof ApiFailure && (failure.status === 409 || failure.status === 422)
					? failure.message
					: 'That name could not be saved. Try again.';
		}
	}));

	const blocked = $derived(claimBlockedReason(status, claiming.isPending));

	function claim(event: SubmitEvent) {
		event.preventDefault();
		const slug = worthProbing(slugDraft);
		if (slug === null) {
			refusal = blocked;
			return;
		}
		refusal = null;
		const name = nameDraft.trim();
		claiming.mutate(name.length === 0 ? { slug } : { name, slug });
	}
</script>

<div class="claim-page page">
	<div class="claim-card">
		<h1 class="claim-title">Name your organisation</h1>
		<p class="claim-lead">
			This is the name your organisation is known by. It is yours alone, it appears in your
			address, and you can change it later.
		</p>

		{#if refusal !== null}
			<div class="claim-banner"><Banner tone="bad">{refusal}</Banner></div>
		{/if}

		<form onsubmit={claim} class="form">
			<!-- No `maxlength`: the hint states the bound, and the server is the
			     one that applies it. -->
			<Field label="Name" id="claim-slug" required hint="Letters, numbers and hyphens.">
				<input
					id="claim-slug"
					name="claim-slug"
					type="text"
					required
					autocomplete="off"
					autocapitalize="none"
					spellcheck="false"
					bind:value={slugDraft}
				/>
			</Field>

			<p class="claim-preview">
				<span class="claim-host">{page.url.host}/</span><span class="claim-said"
					>{status.kind === 'empty' || status.kind === 'invalid' ? 'your-name' : status.slug}</span
				>
			</p>

			<p
				class="claim-verdict {status.kind === 'available'
					? 'claim-good'
					: status.kind === 'taken' || status.kind === 'invalid'
						? 'claim-bad'
						: 'claim-waiting'}"
				aria-live="polite"
			>
				{#if status.kind === 'available'}
					<span class="claim-glyph"><Icon name="circle-check" size={15} /></span>{status.message}
				{:else if status.kind === 'taken'}
					<span class="claim-glyph"><Icon name="circle-x" size={15} /></span>{status.message}
				{:else if status.kind === 'invalid'}
					<span class="claim-glyph"><Icon name="circle-alert" size={15} /></span>{status.message}
				{:else if status.kind === 'unknown'}
					<span class="claim-glyph"><Icon name="circle-question-mark" size={15} /></span
					>{status.message}
				{:else if status.kind === 'checking'}
					<span class="claim-glyph"><Icon name="refresh-cw" size={15} /></span>Checking…
				{/if}
			</p>

			<Field
				label="Display name"
				id="claim-name"
				hint="Optional. What we print. Leave it as it is if it reads right."
			>
				<input
					id="claim-name"
					name="claim-name"
					type="text"
					autocomplete="organization"
					bind:value={nameDraft}
					oninput={() => (nameEdited = true)}
				/>
			</Field>

			<div class="claim-actions">
				{#if dismissable}
					<Button tier="quiet" onclick={onDone}>Not now</Button>
				{/if}
				<Button
					tier="primary"
					type="submit"
					disabled={blocked !== null}
					reason={blocked ?? undefined}
				>
					{claiming.isPending ? 'Saving…' : 'Save name'}
				</Button>
			</div>
		</form>
	</div>
</div>
