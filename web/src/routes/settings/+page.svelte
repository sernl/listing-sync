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
</script>

<h1 class="mb-4 text-xl font-semibold">Settings</h1>

<section class="mb-6 rounded border border-slate-200 bg-white p-4">
	<h2 class="mb-1 font-semibold">Organisation</h2>
	<p class="mb-3 text-sm text-slate-600">
		The name this account trades under. It appears wherever the dashboard names
		your organisation; it is not sent to any marketplace.
	</p>

	{#if organisation.isPending}
		<p class="text-slate-500">Loading…</p>
	{:else if organisation.isError}
		<p class="text-slate-500">The organisation could not be read.</p>
	{:else}
		<form onsubmit={rename} class="flex flex-col gap-2">
			<label class="flex flex-col gap-1 text-sm" for="org-name">
				Name
				<!-- No `maxlength`: it counts UTF-16 code units, so it would
				     silently cut a name of accented or non-Latin letters short of
				     the server's character bound. The verdict states the bound
				     instead. -->
				<input
					id="org-name"
					name="org-name"
					type="text"
					required
					class="rounded border border-slate-300 px-3 py-2"
					bind:value={orgDraft}
				/>
			</label>
			{#if orgRefusal !== null}
				<p class="text-sm text-red-700">{orgRefusal}</p>
			{:else}
				<p class="text-xs text-slate-500">At most {NAME_MAX_CHARS} characters.</p>
			{/if}
			<div>
				<button
					class="rounded bg-slate-900 px-4 py-2 text-sm text-white disabled:opacity-50"
					disabled={renaming.isPending || orgUnchanged || !orgVerdict.accepted}
				>
					{renaming.isPending ? 'Saving…' : 'Save name'}
				</button>
			</div>
		</form>
	{/if}
</section>

<section class="mb-6 rounded border border-slate-200 bg-white p-4">
	<h2 class="mb-1 font-semibold">Profile</h2>
	<p class="mb-3 text-sm text-slate-600">
		Who you are signed in as. Changing your email address is done from the
		sign-in service's own verification flow, not here.
	</p>

	{#if profile.isPending}
		<p class="text-slate-500">Loading…</p>
	{:else if profile.isError || !profile.data}
		<p class="text-slate-500">Your profile could not be read.</p>
	{:else}
		<dl class="mb-3 text-sm">
			<dt class="text-slate-500">Email</dt>
			<dd class="font-medium">
				{profile.data.email}
				{#if !profile.data.emailVerified}
					<span class="ml-2 rounded bg-amber-100 px-2 py-0.5 text-xs text-amber-900">
						unverified
					</span>
				{/if}
			</dd>
		</dl>
		<form onsubmit={saveName} class="flex flex-col gap-2">
			<label class="flex flex-col gap-1 text-sm" for="display-name">
				Display name
				<input
					id="display-name"
					name="display-name"
					type="text"
					required
					autocomplete="name"
					class="rounded border border-slate-300 px-3 py-2"
					bind:value={displayName}
				/>
			</label>
			<div>
				<button
					class="rounded bg-slate-900 px-4 py-2 text-sm text-white disabled:opacity-50"
					disabled={renamingUser.isPending || nameUnchanged || nameBlank}
				>
					{renamingUser.isPending ? 'Saving…' : 'Save name'}
				</button>
			</div>
		</form>
	{/if}
</section>

<section class="rounded border border-slate-200 bg-white p-4">
	<h2 class="mb-1 font-semibold">Passkeys</h2>
	<p class="mb-3 text-sm text-slate-600">
		A passkey signs you in with the same fingerprint, face or PIN that unlocks
		your device. Each one is registered to this account and can be removed
		here.
	</p>

	{#if !supported}
		<p class="text-slate-500">
			This browser does not support passkeys, so none can be registered or
			listed here. Your password and any linked account still work.
		</p>
	{:else}
		{#if passkeys.isPending}
			<p class="text-slate-500">Loading…</p>
		{:else if passkeys.isError}
			<p class="text-slate-500">Your passkeys could not be listed.</p>
		{:else if passkeys.data.length === 0}
			<p class="text-slate-500">No passkeys registered yet.</p>
		{:else}
			<ul class="mb-4 divide-y divide-slate-100 rounded border border-slate-200">
				{#each passkeys.data as passkey (passkey.id)}
					<li class="flex items-center gap-4 px-4 py-3">
						<div class="min-w-0">
							<div class="font-medium">{passkeyLabel(passkey)}</div>
							<div class="text-xs text-slate-500">
								{passkeyReach(passkey)}
								{#if passkey.createdAt}
									Added {new Date(passkey.createdAt).toLocaleDateString()}.
								{/if}
							</div>
						</div>
						<span class="grow"></span>
						<button
							type="button"
							class="rounded border border-red-300 px-3 py-1 text-sm text-red-700 disabled:opacity-50"
							disabled={removing.isPending && removing.variables === passkey.id}
							onclick={() => remove(passkey)}
						>
							{removing.isPending && removing.variables === passkey.id
								? 'Removing…'
								: 'Remove'}
						</button>
					</li>
				{/each}
			</ul>
		{/if}

		<form onsubmit={add} class="flex flex-col gap-2">
			<label class="flex flex-col gap-1 text-sm" for="passkey-label">
				Name for the new passkey <span class="text-slate-500">(optional)</span>
				<input
					id="passkey-label"
					name="passkey-label"
					type="text"
					placeholder="Work laptop"
					class="rounded border border-slate-300 px-3 py-2"
					bind:value={newPasskeyLabel}
				/>
			</label>
			<div>
				<button
					class="rounded border border-slate-300 px-4 py-2 text-sm disabled:opacity-50"
					disabled={registering.isPending}
				>
					{registering.isPending ? 'Waiting for your device…' : 'Register a passkey'}
				</button>
			</div>
		</form>
	{/if}
</section>
