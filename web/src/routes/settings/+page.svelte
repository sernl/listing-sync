<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { ApiFailure, api, type OrgView } from '$lib/api';
	import {
		currentSessionToken,
		listBrowserSessions,
		revokeBrowserSession
	} from '$lib/browser-sessions';
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
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import { type BrowserSession, merge, sessionLabel } from '$lib/device-merge';
	import { agoLabel } from '$lib/elapsed';
	import Field from '$lib/Field.svelte';
	import { NAME_MAX_CHARS, checkOrgName } from '$lib/org-name';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import { passkeyLabel, passkeyReach } from '$lib/passkey-label';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import { toast } from '$lib/toast';
	import '$lib/pages/account/account.css';

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

	const orgBlocked = $derived(
		renaming.isPending
			? 'The name is being saved.'
			: !orgVerdict.accepted
				? orgVerdict.message
				: orgUnchanged
					? 'The name has not changed.'
					: null
	);

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
			toast('info', 'Display name saved.');
		},
		onError: (failure: Error) => {
			toast('error', refusalOf(failure, 'The display name was not saved.'));
		}
	}));

	const nameBlocked = $derived(
		renamingUser.isPending
			? 'The name is being saved.'
			: nameBlank
				? 'A display name cannot be empty.'
				: nameUnchanged
					? 'The name has not changed.'
					: null
	);

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

	// ------------------------------------------------- browser sign-ins

	// The device registry is read here only to subtract: a sign-in already shown
	// against a machine on Marketplaces is not repeated on this panel, and the
	// join that decides which those are needs both halves. The key is the shared
	// one, so this costs no second request when either screen has already read
	// it.
	const registry = createQuery(() => ({
		queryKey: queryKeys.devices,
		queryFn: () => api.devices()
	}));

	const signIns = createQuery(() => ({
		queryKey: queryKeys.browserSessions,
		queryFn: () => listBrowserSessions()
	}));

	const current = createQuery(() => ({
		queryKey: queryKeys.currentSessionToken,
		queryFn: () => currentSessionToken()
	}));

	const joined = $derived(
		merge(registry.data?.devices ?? [], signIns.data ?? [], current.data ?? null)
	);

	const endingSignIn = createMutation(() => ({
		mutationFn: (token: string) => revokeBrowserSession(token),
		onSuccess: async () => {
			toast('info', 'That browser sign-in was ended.');
			await queryClient.invalidateQueries({ queryKey: queryKeys.browserSessions });
		},
		onError: () => {
			toast('error', 'That sign-in was not ended.');
		}
	}));

	// Whether this browser's own session can be picked out of the list at all.
	// `currentSessionToken` resolves to null on a refused read rather than
	// throwing, so a failed read is indistinguishable from a signed-out one
	// here and every row's `isCurrent` comes back false. Without this, ending
	// the session the seller is reading the page in warned them about somebody
	// else's browser and signed them out.
	const currentKnown = $derived(current.isSuccess && current.data !== null);

	function endSignIn(session: BrowserSession, isCurrent: boolean) {
		const sure = confirm(
			isCurrent
				? 'End this sign-in? You are using it right now, so you will be signed out of this browser.'
				: currentKnown
					? 'End this browser sign-in? That browser will have to sign in again.'
					: 'End this browser sign-in? We could not tell which of these is the browser you are ' +
						'using, so if it is this one you will be signed out here.'
		);
		if (sure) {
			endingSignIn.mutate(session.token);
		}
	}
</script>

<div class="page">
	<PageHead
		icon="sliders-horizontal"
		title="Preferences"
		description="Your organisation, your profile, and the ways you sign in."
	>
		{#snippet aside()}
			<Button tier="quiet" icon="credit-card" href="/settings/subscription">Subscription</Button>
		{/snippet}
	</PageHead>

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
				<!-- No `maxlength`: it counts UTF-16 code units, so it would silently
				     cut a name of accented or non-Latin letters short of the server's
				     character bound. The hint states the bound instead. -->
				<Field
					label="Name"
					id="org-name"
					required
					hint={orgRefusal === null ? `At most ${NAME_MAX_CHARS} characters.` : undefined}
				>
					<input id="org-name" name="org-name" type="text" required bind:value={orgDraft} />
				</Field>
				{#if orgRefusal !== null}
					<Banner tone="bad">{orgRefusal}</Banner>
				{/if}
				<div class="actions">
					<Button tier="primary" type="submit" disabled={orgBlocked !== null} reason={orgBlocked ?? undefined}>
						{renaming.isPending ? 'Saving…' : 'Save organisation name'}
					</Button>
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
			<dl class="acct-detail">
				<dt>Email</dt>
				<dd>
					{profile.data.email}
					{#if !profile.data.emailVerified}
						<StatusPill tone="run" label="unverified" />
					{/if}
				</dd>
			</dl>
			<form onsubmit={saveName} class="form">
				<Field label="Display name" id="display-name" required>
					<input
						id="display-name"
						name="display-name"
						type="text"
						required
						autocomplete="name"
						bind:value={displayName}
					/>
				</Field>
				<div class="actions">
					<Button
						tier="primary"
						type="submit"
						disabled={nameBlocked !== null}
						reason={nameBlocked ?? undefined}
					>
						{renamingUser.isPending ? 'Saving…' : 'Save display name'}
					</Button>
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
					<div class="acct-state-row">
						<span class="who">
							<span class="t">{passkeyLabel(passkey)}</span>
							<span class="why">
								{passkeyReach(passkey)}
								{#if passkey.createdAt}
									Added {new Date(passkey.createdAt).toLocaleDateString()}.
								{/if}
							</span>
						</span>
						<Button
							tier="outline"
							danger
							small
							disabled={removing.isPending && removing.variables === passkey.id}
							reason={removing.isPending && removing.variables === passkey.id
								? 'This passkey is being removed.'
								: undefined}
							onclick={() => remove(passkey)}
						>
							{removing.isPending && removing.variables === passkey.id ? 'Removing…' : 'Remove'}
						</Button>
					</div>
				{/each}
			{/if}

			<form onsubmit={add} class="form spaced">
				<Field label="Name for the new passkey" id="passkey-label" hint="Optional.">
					<input
						id="passkey-label"
						name="passkey-label"
						type="text"
						placeholder="Work laptop"
						bind:value={newPasskeyLabel}
					/>
				</Field>
				<div class="actions">
					<Button
						tier="outline"
						type="submit"
						disabled={registering.isPending}
						reason={registering.isPending ? 'Your device is being asked for a passkey.' : undefined}
					>
						{registering.isPending ? 'Waiting for your device…' : 'Register a passkey'}
					</Button>
				</div>
			</form>
		{/if}
	</Panel>

	<Panel
		title="Browser sign-ins"
		description="Where this account is signed in to the console. These are sign-ins to us, not to any marketplace; the identity service records only the address and the browser each was made from, which is why some cannot be pinned to a machine."
	>
		<div class="acct-state-row">
			<span class="who">
				<span class="t">Machines</span>
				<span class="why">
					A machine you no longer use is signed out on Marketplaces, beside the logins it holds.
				</span>
			</span>
			<Button tier="outline" small href="/marketplaces">Open</Button>
		</div>

		<!-- All three reads, not just the list. `joined` merges the browser
		     sessions with the device registry, so a failed registry read empties
		     the device half and lists every session here as an orphan. -->
		{#if signIns.isPending || registry.isPending || current.isPending}
			<p class="quiet">Loading…</p>
		{:else if signIns.isError || registry.isError}
			<p class="quiet">Your browser sign-ins could not be listed.</p>
		{:else if joined.orphans.length === 0}
			<p class="quiet">
				Every browser sign-in on this account is shown against a machine on Marketplaces.
			</p>
		{:else}
			{#each joined.orphans as orphan (orphan.session.token)}
				<div class="acct-state-row">
					<span class="who">
						<span class="t">
							{sessionLabel(orphan.session)}
							{#if orphan.isCurrent}<StatusPill tone="ok" label="this browser" />{/if}
						</span>
						{#if orphan.session.createdAt}
							<span class="why">
								Signed in {agoLabel(new Date(orphan.session.createdAt).getTime(), Date.now())}
							</span>
						{/if}
					</span>
					<Button
						tier="outline"
						danger
						small
						disabled={endingSignIn.isPending && endingSignIn.variables === orphan.session.token}
						reason={endingSignIn.isPending && endingSignIn.variables === orphan.session.token
							? 'This sign-in is being ended.'
							: undefined}
						onclick={() => endSignIn(orphan.session, orphan.isCurrent)}
					>
						{endingSignIn.isPending && endingSignIn.variables === orphan.session.token
							? 'Ending…'
							: 'End sign-in'}
					</Button>
				</div>
			{/each}
		{/if}
		<p class="foot-note">
			Ending a sign-in takes effect on the next request that browser makes; the session cache
			that would otherwise delay it is switched off for this account.
			{#if !currentKnown}
				We could not tell which of these is the browser you are using, so none is marked.
			{/if}
		</p>
	</Panel>
</div>
