<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { ApiFailure, api, type OrgView } from '$lib/api';
	import {
		ALREADY_REGISTERED,
		AuthFailure,
		CEREMONY_ABORTED,
		deletePasskey,
		identity,
		listPasskeys,
		passkeysSupported,
		registerPasskey,
		updateDisplayName,
		type Identity,
		type PasskeyRecord
	} from '$lib/auth-client';
	import { NAME_MAX_CHARS, checkOrgName } from '$lib/org-name';
	import { PADDLE_CONFIG, openCheckout, subscriptionTone } from '$lib/paddle';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import { passkeyLabel, passkeyReach } from '$lib/passkey-label';
	import { queryKeys } from '$lib/query';
	import { toast } from '$lib/toast';

	const queryClient = useQueryClient();

	function refusalOf(failure: Error, fallback: string): string {
		return failure instanceof AuthFailure ? failure.message : fallback;
	}

	// --------------------------------------------------------- organisation

	const organisation = createQuery(() => ({
		queryKey: queryKeys.org,
		queryFn: () => api.org()
	}));

	let orgDraft = $state('');
	let orgSeeded = $state(false);
	let orgRefusal = $state<string | null>(null);

	// Seeded once rather than mirrored: a refetch that arrives while the seller
	// is typing must not overwrite what they typed.
	$effect(() => {
		const stored = organisation.data?.name;
		if (stored !== undefined && !orgSeeded) {
			orgDraft = stored;
			orgSeeded = true;
		}
	});

	const orgVerdict = $derived(checkOrgName(orgDraft));
	const orgUnchanged = $derived(
		orgVerdict.accepted && orgVerdict.name === organisation.data?.name
	);

	const renaming = createMutation(() => ({
		mutationFn: (name: string) => api.renameOrg(name),
		onSuccess: (stored: OrgView) => {
			// The server trims on the way in and answers with what it kept, so
			// the cache and the field both take the stored value rather than
			// the submitted one.
			queryClient.setQueryData(queryKeys.org, stored);
			orgDraft = stored.name;
			orgRefusal = null;
			toast('info', 'Organisation name saved.');
		},
		onError: (failure: Error) => {
			// A 422 is about the name in the field, so it is answered beside the
			// field. Anything else is about the request and goes to the toasts.
			if (failure instanceof ApiFailure && failure.status === 422) {
				orgRefusal = failure.message;
				return;
			}
			toast('error', 'The organisation name was not saved.');
		}
	}));

	function rename(event: SubmitEvent) {
		event.preventDefault();
		if (!orgVerdict.accepted) {
			orgRefusal = orgVerdict.message;
			return;
		}
		orgRefusal = null;
		renaming.mutate(orgVerdict.name);
	}

	// -------------------------------------------------------------- profile

	const profile = createQuery(() => ({
		queryKey: queryKeys.identity,
		queryFn: () => identity()
	}));

	let displayName = $state('');
	let nameSeeded = $state(false);

	$effect(() => {
		const stored = profile.data?.name;
		if (stored !== undefined && !nameSeeded) {
			displayName = stored;
			nameSeeded = true;
		}
	});

	const nameUnchanged = $derived(displayName.trim() === profile.data?.name);
	const nameBlank = $derived(displayName.trim().length === 0);

	const renamingUser = createMutation(() => ({
		mutationFn: (name: string) => updateDisplayName(name),
		onSuccess: async () => {
			// `/update-user` answers `{ status: true }` rather than the updated
			// user, so the stored name is read back from the session instead of
			// being assumed from the field.
			await queryClient.invalidateQueries({ queryKey: queryKeys.identity });
			const stored = queryClient.getQueryData<Identity | null>(queryKeys.identity);
			if (stored) {
				displayName = stored.name;
			}
			toast('info', 'Name saved.');
		},
		onError: (failure: Error) => {
			toast('error', refusalOf(failure, 'The name was not saved.'));
		}
	}));

	function saveName(event: SubmitEvent) {
		event.preventDefault();
		renamingUser.mutate(displayName.trim());
	}

	// ------------------------------------------------------------- passkeys

	const supported = passkeysSupported();

	const passkeys = createQuery(() => ({
		queryKey: queryKeys.passkeys,
		queryFn: () => listPasskeys(),
		enabled: supported
	}));

	let newPasskeyLabel = $state('');

	const registering = createMutation(() => ({
		mutationFn: (label: string) => registerPasskey(label),
		onSuccess: async () => {
			newPasskeyLabel = '';
			await queryClient.invalidateQueries({ queryKey: queryKeys.passkeys });
			toast('info', 'Passkey registered.');
		},
		onError: (failure: Error) => {
			const code = failure instanceof AuthFailure ? failure.code : undefined;
			if (code === CEREMONY_ABORTED) {
				// Dismissing the browser's prompt is a decision, not a fault.
				toast('info', 'Passkey registration cancelled. Nothing changed.');
				return;
			}
			if (code === ALREADY_REGISTERED) {
				toast('error', 'That authenticator is already registered on this account.');
				return;
			}
			toast('error', refusalOf(failure, 'The passkey was not registered.'));
		}
	}));

	const removing = createMutation(() => ({
		mutationFn: (id: string) => deletePasskey(id),
		onSuccess: async () => {
			await queryClient.invalidateQueries({ queryKey: queryKeys.passkeys });
			toast('info', 'Passkey removed.');
		},
		onError: (failure: Error) => {
			toast('error', refusalOf(failure, 'The passkey was not removed.'));
		}
	}));

	function add(event: SubmitEvent) {
		event.preventDefault();
		registering.mutate(newPasskeyLabel);
	}

	function remove(passkey: PasskeyRecord) {
		const sure = confirm(
			`Remove "${passkeyLabel(passkey)}"? Signing in with that passkey stops ` +
				'working immediately. Your password and any other passkeys are unaffected.'
		);
		if (!sure) {
			return;
		}
		removing.mutate(passkey.id);
	}

	// -------------------------------------------------------------- billing

	const billing = createQuery(() => ({
		queryKey: queryKeys.billing,
		queryFn: () => api.billing()
	}));

	const subscription = $derived(billing.data?.subscription ?? null);

	// The button renders only where the build was given Paddle's client token
	// and price. With neither, the panel states what is recorded and loads
	// nothing from Paddle at all.
	const checkout = PADDLE_CONFIG;
	let opening = $state(false);

	async function subscribe() {
		const org = organisation.data?.id;
		if (checkout === null || org === undefined) {
			return;
		}
		opening = true;
		try {
			await openCheckout(checkout, org, profile.data?.email);
		} catch {
			toast('error', 'The checkout could not be opened.');
		} finally {
			opening = false;
		}
	}
</script>

<div class="page">
	<PageHead
		icon="⚙"
		title="Settings"
		description="Organisation, profile, passkeys and billing."
	/>

	<Panel
		title="Organisation"
		description="The name this account trades under. It appears wherever the console names your organisation; it is not sent to any marketplace."
	>
		{#if organisation.isPending}
			<p class="quiet">Loading…</p>
		{:else if organisation.isError}
			<p class="quiet">The organisation could not be read.</p>
		{:else}
			<form onsubmit={rename} class="form">
				<label class="field" for="org-name">
					Name
					<!-- No `maxlength`: it counts UTF-16 code units, so it would
					     silently cut a name of accented or non-Latin letters short of
					     the server's character bound. The verdict states the bound
					     instead. -->
					<input id="org-name" name="org-name" type="text" required bind:value={orgDraft} />
				</label>
				{#if orgRefusal !== null}
					<p class="refusal">{orgRefusal}</p>
				{:else}
					<p class="foot-note">At most {NAME_MAX_CHARS} characters.</p>
				{/if}
				<div class="actions">
					<button
						class="cta"
						disabled={renaming.isPending || orgUnchanged || !orgVerdict.accepted}
					>
						{renaming.isPending ? 'Saving…' : 'Save name'}
					</button>
				</div>
			</form>
		{/if}
	</Panel>

	<Panel
		title="Profile"
		description="Who you are signed in as. Changing your email address is done from the sign-in service's own verification flow, not here."
	>
		{#if profile.isPending}
			<p class="quiet">Loading…</p>
		{:else if profile.isError || !profile.data}
			<p class="quiet">Your profile could not be read.</p>
		{:else}
			<dl class="facts">
				<dt>Email</dt>
				<dd>
					{profile.data.email}
					{#if !profile.data.emailVerified}
						<span class="pill run">unverified</span>
					{/if}
				</dd>
			</dl>
			<form onsubmit={saveName} class="form">
				<label class="field" for="display-name">
					Display name
					<input
						id="display-name"
						name="display-name"
						type="text"
						required
						autocomplete="name"
						bind:value={displayName}
					/>
				</label>
				<div class="actions">
					<button class="cta" disabled={renamingUser.isPending || nameUnchanged || nameBlank}>
						{renamingUser.isPending ? 'Saving…' : 'Save name'}
					</button>
				</div>
			</form>
		{/if}
	</Panel>

	<Panel
		title="Passkeys"
		description="A passkey signs you in with the same fingerprint, face or PIN that unlocks your device. Each one is registered to this account and can be removed here."
	>
		{#if !supported}
			<p class="quiet">
				This browser does not support passkeys, so none can be registered or listed here. Your
				password and any linked account still work.
			</p>
		{:else}
			{#if passkeys.isPending}
				<p class="quiet">Loading…</p>
			{:else if passkeys.isError}
				<p class="quiet">Your passkeys could not be listed.</p>
			{:else if passkeys.data.length === 0}
				<p class="quiet">No passkeys registered yet.</p>
			{:else}
				{#each passkeys.data as passkey (passkey.id)}
					<div class="row">
						<span class="what">
							<span class="t">{passkeyLabel(passkey)}</span>
							<span class="s">
								{passkeyReach(passkey)}
								{#if passkey.createdAt}
									Added {new Date(passkey.createdAt).toLocaleDateString()}.
								{/if}
							</span>
						</span>
						<span class="grow"></span>
						<button
							type="button"
							class="btn small danger"
							disabled={removing.isPending && removing.variables === passkey.id}
							onclick={() => remove(passkey)}
						>
							{removing.isPending && removing.variables === passkey.id
								? 'Removing…'
								: 'Remove'}
						</button>
					</div>
				{/each}
			{/if}

			<form onsubmit={add} class="form spaced">
				<label class="field" for="passkey-label">
					Name for the new passkey <span class="hint">(optional)</span>
					<input
						id="passkey-label"
						name="passkey-label"
						type="text"
						placeholder="Work laptop"
						bind:value={newPasskeyLabel}
					/>
				</label>
				<div class="actions">
					<button class="btn" disabled={registering.isPending}>
						{registering.isPending ? 'Waiting for your device…' : 'Register a passkey'}
					</button>
				</div>
			</form>
		{/if}
	</Panel>

	<Panel
		title="Billing"
		description="What Paddle has recorded for this organisation. Nothing here gates a feature today; every tenant is served exactly as before."
	>
		{#if billing.isPending}
			<p class="quiet">Loading…</p>
		{:else if billing.isError}
			<p class="quiet">Your billing could not be read.</p>
		{:else if subscription === null}
			<div class="placeholder">
				<span class="big" aria-hidden="true">◇</span>
				<b>No subscription.</b>
				<p>
					This organisation has never reached checkout, which is a different fact from a
					cancelled subscription — that one would be shown here with its status.
				</p>
			</div>
		{:else}
			<dl class="facts">
				<dt>Status</dt>
				<dd>
					<span class="pill {subscriptionTone(subscription.status)}">{subscription.status}</span>
				</dd>
				<dt>Current period ends</dt>
				<dd>
					{subscription.current_period_end === null
						? 'Paddle recorded no billing period'
						: new Date(subscription.current_period_end).toLocaleDateString()}
				</dd>
				<dt>Last recorded</dt>
				<dd>{new Date(subscription.occurred_at).toLocaleString()}</dd>
			</dl>
			<p class="foot-note">
				Subscription <span class="mono">{subscription.paddle_subscription_id}</span> ·
				customer <span class="mono">{subscription.paddle_customer_id}</span>. The status is
				Paddle's own word for it, passed through rather than translated.
			</p>
		{/if}

		{#if checkout !== null}
			<div class="actions">
				<button
					class="cta"
					type="button"
					disabled={opening || organisation.data === undefined}
					onclick={subscribe}
				>
					{opening ? 'Opening…' : subscription === null ? 'Subscribe' : 'Change plan'}
				</button>
			</div>
		{/if}
	</Panel>
</div>
