<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { page } from '$app/state';
	import {
		ApiFailure,
		api,
		avatarSrc,
		type NotifyPreferences,
		type OrgView,
		type ProfileView
	} from '$lib/api';
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
	import { initialsOf } from '$lib/nav';
	import { NAME_MAX_CHARS, checkOrgName } from '$lib/org-name';
	import Note from '$lib/Note.svelte';
	import { checkOrgSlug } from '$lib/org-slug';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import { passkeyLabel, passkeyReach } from '$lib/passkey-label';
	import { queryKeys } from '$lib/query';
	import { signOut } from '$lib/sign-out';
	import StatusPill from '$lib/StatusPill.svelte';
	import Toggle from '$lib/Toggle.svelte';
	import { toast } from '$lib/toast';
	import { NOT_REMOVED, avatarRefusal, pictureRefusal } from '$lib/pages/account/avatar';
	import Preferences from '$lib/pages/account/Preferences.svelte';
	import Machines from '$lib/pages/account/Machines.svelte';
	import { STORAGE_NOT_RECLAIMED } from '$lib/pages/resources/files';
	import { historyLines, permissionRows, withdrawPrompt } from '$lib/pages/account/permissions';
	import ConsentDialog from '$lib/pages/marketplaces/ConsentDialog.svelte';
	import type { Marketplace } from '$lib/generated/vocab';
	import '$lib/pages/account/account.css';

	const queryClient = useQueryClient();
	const now = Date.now();

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
		mutationFn: (name: string) => api.updateOrg({ name }),
		onSuccess: (stored: OrgView) => {
			// The server trims on the way in and answers with what it kept, so
			// the cache and the field both take the stored value rather than
			// the submitted one.
			queryClient.setQueryData(queryKeys.org, stored);
			orgDraft = stored.name;
			orgRefusal = null;
			toast('info', 'Account name saved.');
		},
		onError: (failure: Error) => {
			// A 422 is about the name in the field, so it is answered beside the
			// field. Anything else is about the request and goes to the toasts.
			if (failure instanceof ApiFailure && failure.status === 422) {
				orgRefusal = failure.message;
				return;
			}
			toast('error', 'Your account name was not saved. Try again.');
		}
	}));

	const orgBlocked = $derived(
		renaming.isPending
			? 'Saving your name.'
			: !orgVerdict.accepted
				? orgVerdict.message
				: orgUnchanged
					? 'Change the name to save it.'
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

	// -------------------------------------------------------- notifications

	// Per user rather than per organisation, because the address the mail goes
	// to is the user's own. There is no field with which either call could name
	// another user.
	const notifyPrefs = createQuery(() => ({
		queryKey: queryKeys.notifyPreferences,
		queryFn: () => api.notifyPreferences()
	}));

	/** The pending setting while a write is in flight, so the switch moves
	 *  under the thumb rather than after the round trip, and null the rest of
	 *  the time, when the stored value is what the switch shows. No default:
	 *  the switch is drawn only where the read succeeded, so there is no state
	 *  in which this page would have to guess which way it is set. */
	let notifyDraft = $state<boolean | null>(null);

	const settingNotifyEmail = createMutation(() => ({
		mutationFn: (value: boolean) => api.setNotifyPreferences(value),
		onMutate: (value: boolean) => {
			notifyDraft = value;
		},
		onSuccess: (stored: NotifyPreferences) => {
			// The stored value rather than the submitted one, so the switch
			// shows what the server holds.
			queryClient.setQueryData(queryKeys.notifyPreferences, stored);
			notifyDraft = null;
			toast('info', stored.notify_email ? 'Emails on.' : 'Emails off.');
		},
		onError: () => {
			notifyDraft = null;
			toast('error', 'That setting was not saved.');
		}
	}));

	// ------------------------------------------------------- the org's name

	// A second form rather than a second field on the one above, and the reason
	// is attribution: one PATCH carrying both fields answers one refusal, and
	// nothing in that answer says which field it was about. One field per form
	// means a 422 and a 409 both land beside the control they are about.

	let slugDraft = $state('');
	let slugSeeded = $state(false);
	let slugRefusal = $state<string | null>(null);

	$effect(() => {
		const stored = organisation.data?.slug;
		if (stored !== undefined && !slugSeeded) {
			slugDraft = stored ?? '';
			slugSeeded = true;
		}
	});

	const slugVerdict = $derived(checkOrgSlug(slugDraft));
	const slugUnchanged = $derived(
		slugVerdict.accepted && slugVerdict.slug === organisation.data?.slug
	);

	const claiming = createMutation(() => ({
		mutationFn: (slug: string) => api.updateOrg({ slug }),
		onSuccess: (stored: OrgView) => {
			queryClient.setQueryData(queryKeys.org, stored);
			slugDraft = stored.slug ?? '';
			slugRefusal = null;
			toast('info', 'Account name saved.');
		},
		onError: (failure: Error) => {
			// A 409 says another organisation holds the name and a 422 says the
			// name is malformed or reserved. Both are about the field, so both
			// answer beside it -- the 409 included, because the availability
			// check is advisory and only the server's index decides.
			if (failure instanceof ApiFailure && (failure.status === 409 || failure.status === 422)) {
				slugRefusal = failure.message;
				return;
			}
			toast('error', 'Your account name was not saved. Try again.');
		}
	}));

	const slugBlocked = $derived(
		claiming.isPending
			? 'Saving your name.'
			: !slugVerdict.accepted
				? slugVerdict.message
				: slugUnchanged
					? 'Change the name to save it.'
					: null
	);

	function claim(event: SubmitEvent) {
		event.preventDefault();
		if (!slugVerdict.accepted) {
			slugRefusal = slugVerdict.message;
			return;
		}
		slugRefusal = null;
		claiming.mutate(slugVerdict.slug);
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
			? 'Saving your name.'
			: nameBlank
				? 'Enter a display name.'
				: nameUnchanged
					? 'Change the name to save it.'
					: null
	);

	function saveName(event: SubmitEvent) {
		event.preventDefault();
		renamingUser.mutate(displayName.trim());
	}

	// ------------------------------------------------------------- picture

	// The picture is the domain's rather than the identity service's: it is
	// read and written on our own API, so its block draws whether or not the
	// identity read above succeeded.
	const picture = createQuery(() => ({
		queryKey: queryKeys.profile,
		queryFn: () => api.profile()
	}));

	let avatarSending = $state(false);
	let avatarRefused = $state<string | null>(null);
	// The picture whose bytes would not draw, held as its address so a later
	// picture clears it by being a different address.
	let unshowable = $state<string | null>(null);
	const avatarShown = $derived.by(() => {
		const src = avatarSrc(picture.data);
		return src === unshowable ? null : src;
	});
	const hasPicture = $derived(picture.data !== undefined && picture.data.avatar_hash !== null);
	const avatarInitials = $derived(initialsOf(organisation.data?.name));

	/** The bytes go where every picture goes, slot-bound so the server refuses
	 *  a worksheet before it is sealed, and the profile write names the handle
	 *  that upload answered; the cache takes the server's answer rather than
	 *  the handle this client sent. */
	async function chooseAvatar(event: Event & { currentTarget: HTMLInputElement }) {
		const file = event.currentTarget.files?.[0];
		event.currentTarget.value = '';
		if (!file) {
			return;
		}
		const unusable = pictureRefusal(file.type);
		if (unusable !== null) {
			avatarRefused = unusable;
			return;
		}
		avatarSending = true;
		avatarRefused = null;
		try {
			const landed = await api.upload(file, 'keep_whole', undefined, 'image');
			const handle = landed.payload[0]?.hash;
			if (handle === undefined) {
				avatarRefused = 'That picture could not be attached. Try again.';
				return;
			}
			const stored = await api.setAvatar(handle);
			queryClient.setQueryData(queryKeys.profile, stored);
			toast('info', 'Profile picture saved.');
		} catch (failure) {
			avatarRefused = avatarRefusal(failure);
		} finally {
			avatarSending = false;
		}
	}

	const removingAvatar = createMutation(() => ({
		mutationFn: () => api.clearAvatar(),
		onSuccess: (stored: ProfileView) => {
			queryClient.setQueryData(queryKeys.profile, stored);
			avatarRefused = null;
			toast('info', 'Profile picture removed.');
		},
		onError: () => {
			toast('error', NOT_REMOVED);
		}
	}));

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
			toast('info', 'Passkey added.');
		},
		onError: (failure: Error) => {
			const code = failure instanceof AuthFailure ? failure.code : undefined;
			if (code === CEREMONY_ABORTED) {
				// Dismissing the browser's prompt is a decision, not a fault.
				toast('info', 'Passkey not added. Nothing changed.');
				return;
			}
			if (code === ALREADY_REGISTERED) {
				toast('error', 'That device already has a passkey for this account.');
				return;
			}
			toast('error', refusalOf(failure, 'The passkey was not added. Try again.'));
		}
	}));

	const removing = createMutation(() => ({
		mutationFn: (id: string) => deletePasskey(id),
		onSuccess: async () => {
			await queryClient.invalidateQueries({ queryKey: queryKeys.passkeys });
			toast('info', 'Passkey removed.');
		},
		onError: (failure: Error) => {
			toast('error', refusalOf(failure, 'The passkey was not removed. Try again.'));
		}
	}));

	function add(event: SubmitEvent) {
		event.preventDefault();
		registering.mutate(newPasskeyLabel);
	}

	function remove(passkey: PasskeyRecord) {
		const sure = confirm(
			`Remove "${passkeyLabel(passkey)}"? You can no longer sign in with it. ` +
				'Your password and other passkeys still work.'
		);
		if (!sure) {
			return;
		}
		removing.mutate(passkey.id);
	}

	// ------------------------------------------------- browser sign-ins

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
			toast('info', 'That browser is signed out.');
			await queryClient.invalidateQueries({ queryKey: queryKeys.browserSessions });
		},
		onError: () => {
			toast('error', 'That browser was not signed out. Try again.');
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
				? 'End this sign-in? You are using it now, so you will be signed out of this browser.'
				: currentKnown
					? 'End this browser sign-in? That browser will need to sign in again.'
					: 'End this browser sign-in? We cannot tell if it is this browser, so you may be ' +
						'signed out here.'
		);
		if (sure) {
			endingSignIn.mutate(session.token);
		}
	}

	// --------------------------------------------- marketplace permissions

	// The seller-device consent record, granted and withdrawn here once per
	// organisation. The Marketplaces page asks for the same grant at Connect;
	// this panel is where a seller reads the record and takes it back.
	const consents = createQuery(() => ({
		queryKey: queryKeys.consents,
		queryFn: () => api.consents()
	}));
	const permissions = $derived(permissionRows(consents.data));
	const record = $derived(historyLines(consents.data?.consents ?? []));
	let grantingFor = $state<Marketplace | null>(null);

	const withdrawing = createMutation(() => ({
		mutationFn: (marketplace: Marketplace) => api.withdrawConsent(marketplace),
		onSuccess: async () => {
			toast('info', 'Permission withdrawn.');
			await Promise.all([
				queryClient.invalidateQueries({ queryKey: queryKeys.consents }),
				queryClient.invalidateQueries({ queryKey: queryKeys.connections })
			]);
		},
		onError: (failure: Error) => {
			toast('error', failure instanceof ApiFailure ? failure.message : 'The permission was not withdrawn. Try again.');
		}
	}));

	function withdraw(marketplace: Marketplace, name: string) {
		if (!confirm(withdrawPrompt(name))) {
			return;
		}
		withdrawing.mutate(marketplace);
	}
</script>

<div class="page">
	<PageHead
		icon="sliders-horizontal"
		title="Account settings"
		description="Your account name, profile and ways to sign in."
		guide="account"
	>
		{#snippet aside()}
			<Button tier="quiet" icon="credit-card" href="/settings/subscription">Plan and moves</Button>
			<!-- The shell's account nav-card carries the other one, and `shell.css`
			     hides that card below 620px, so this is the whole of signing out on
			     a phone. Both run `signOut`. -->
			<Button tier="quiet" icon="log-out" onclick={() => signOut(queryClient)}>Log out</Button>
		{/snippet}
	</PageHead>

	<Panel
		title="Account name"
		description="What your account is called."
	>
		{#if organisation.isPending}
			<p class="quiet">Loading…</p>
		{:else if organisation.isError}
			<p class="quiet">We could not load your account name.</p>
		{:else}
			<form onsubmit={rename} class="form">
				<!-- No `maxlength`: it counts UTF-16 code units, so it would silently
				     cut a name of accented or non-Latin letters short of the server's
				     character bound. The hint states the bound instead. -->
				<Field
					label="Display name"
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
						{renaming.isPending ? 'Saving…' : 'Save display name'}
					</Button>
				</div>
			</form>

			<hr class="acct-rule" />

			<form onsubmit={claim} class="form">
				<Field
					label="Name"
					id="org-slug"
					required
					hint={slugRefusal === null ? 'Letters, numbers and hyphens.' : undefined}
				>
					<input
						id="org-slug"
						name="org-slug"
						type="text"
						required
						autocomplete="off"
						autocapitalize="none"
						spellcheck="false"
						bind:value={slugDraft}
					/>
				</Field>
				<p class="acct-slug-preview">
					<span class="acct-slug-host">{page.url.host}/</span><span class="acct-slug-said"
						>{slugVerdict.accepted ? slugVerdict.slug : 'your-name'}</span
					>
				</p>
				{#if slugRefusal !== null}
					<Banner tone="bad">{slugRefusal}</Banner>
				{/if}
				<div class="actions">
					<Button
						tier="primary"
						type="submit"
						disabled={slugBlocked !== null}
						reason={slugBlocked ?? undefined}
					>
						{claiming.isPending
							? 'Saving…'
							: organisation.data?.slug === null
								? 'Choose name'
								: 'Save name'}
					</Button>
				</div>
			</form>
		{/if}
	</Panel>

	<Panel
		title="Profile"
		description="Who you are signed in as."
	>
		<div class="acct-avatar">
			<!-- Decoration beside the control that names it: the tile shows the
			     picture or the initials the shell would draw in its place. -->
			<span class="acct-avatar-tile" aria-hidden="true">
				{#if avatarShown !== null}
					<img
						class="acct-avatar-img"
						src={avatarShown}
						alt=""
						onerror={() => (unshowable = avatarShown)}
					/>
				{:else}
					{avatarInitials}
				{/if}
			</span>
			<div class="acct-avatar-body">
				<label class="drop" for="avatar-file">
					<b>
						{avatarSending
							? 'Uploading…'
							: hasPicture
								? 'Choose a different picture'
								: 'Choose a picture'}
					</b>
					A JPEG, PNG or GIF, shown beside your account name.
					<input
						id="avatar-file"
						type="file"
						accept="image/png,image/jpeg,image/gif"
						disabled={avatarSending}
						onchange={chooseAvatar}
					/>
				</label>
				{#if avatarRefused !== null}
					<Banner tone="bad">{avatarRefused}</Banner>
				{/if}
				{#if hasPicture}
					<div class="actions">
						<Button
							tier="outline"
							danger
							icon="trash-2"
							small
							disabled={avatarSending || removingAvatar.isPending}
							reason={avatarSending
								? 'Wait for the upload to finish.'
								: removingAvatar.isPending
									? 'Removing the picture.'
									: undefined}
							onclick={() => removingAvatar.mutate()}
						>
							{removingAvatar.isPending ? 'Removing…' : 'Remove picture'}
						</Button>
					</div>
					<Note>{STORAGE_NOT_RECLAIMED}</Note>
				{/if}
			</div>
		</div>

		<hr class="acct-rule" />

		{#if profile.isPending}
			<p class="quiet">Loading…</p>
		{:else if profile.isError || !profile.data}
			<p class="quiet">We could not load your profile.</p>
		{:else}
			<dl class="acct-detail">
				<dt>Email</dt>
				<dd>
					{profile.data.email}
					{#if !profile.data.emailVerified}
						<StatusPill tone="run" label="not verified" />
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
		title="Notifications"
		description="Choose when we email you."
	>
		{#if notifyPrefs.isPending}
			<p class="quiet">Loading…</p>
		{:else if notifyPrefs.isError || !notifyPrefs.data}
			<p class="quiet">We could not load your email settings.</p>
		{:else}
			<Toggle
				label="Email me when an import or move finishes"
				checked={notifyDraft ?? notifyPrefs.data.notify_email}
				disabled={settingNotifyEmail.isPending}
				onchange={(value) => settingNotifyEmail.mutate(value)}
			/>
			<!-- The address comes from the profile read above rather than from
			     this setting's own answer: the domain database holds no seller
			     address by design, so the identity service is the only thing
			     that knows one. -->
			<Note icon="circle-user">
				{#if profile.data}
					Emails go to {profile.data.email}.
				{:else}
					Emails go to the address you signed up with.
				{/if}
			</Note>
		{/if}
	</Panel>

	<Panel
		title="Passkeys"
		description="A passkey signs you in with the fingerprint, face or PIN that unlocks your device."
	>
		{#if !supported}
			<p class="quiet">
				This browser does not support passkeys. Use your password instead.
			</p>
		{:else}
			{#if passkeys.isPending}
				<p class="quiet">Loading…</p>
			{:else if passkeys.isError}
				<p class="quiet">We could not load your passkeys.</p>
			{:else if passkeys.data.length === 0}
				<p class="quiet">No passkeys yet.</p>
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
							icon="trash-2"
							small
							disabled={removing.isPending && removing.variables === passkey.id}
							reason={removing.isPending && removing.variables === passkey.id
								? 'Removing this passkey.'
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
						icon="shield-check"
						type="submit"
						disabled={registering.isPending}
						reason={registering.isPending ? 'Follow the prompt on your device.' : undefined}
					>
						{registering.isPending ? 'Waiting for your device…' : 'Add a passkey'}
					</Button>
				</div>
			</form>
		{/if}
	</Panel>

	<Panel
		title="Browser sign-ins"
		description="Browsers signed in to your Teachouse account."
	>
		{#if signIns.isPending || current.isPending}
			<p class="quiet">Loading…</p>
		{:else if signIns.isError}
			<p class="quiet">We could not load your browser sign-ins.</p>
		{:else if (signIns.data ?? []).length === 0}
			<p class="quiet">No browser sign-ins.</p>
		{:else}
			{#each signIns.data ?? [] as session (session.token)}
				{@const isCurrent = session.token === current.data}
				<div class="acct-state-row">
					<span class="who">
						<span class="t">
							{sessionLabel(session)}
							{#if isCurrent}<StatusPill tone="ok" label="this browser" />{/if}
						</span>
						{#if session.createdAt}
							<span class="why">
								Signed in {agoLabel(new Date(session.createdAt).getTime(), now)}
							</span>
						{/if}
					</span>
					<Button
						tier="outline"
						danger
						icon="log-out"
						small
						disabled={endingSignIn.isPending && endingSignIn.variables === session.token}
						reason={endingSignIn.isPending && endingSignIn.variables === session.token
							? 'Ending this sign-in.'
							: undefined}
						onclick={() => endSignIn(session, isCurrent)}
					>
						{endingSignIn.isPending && endingSignIn.variables === session.token
							? 'Ending…'
							: 'End sign-in'}
					</Button>
				</div>
			{/each}
		{/if}
		<Note>
			{currentKnown
				? 'An ended sign-in stops working the next time that browser opens Teachouse.'
				: 'We could not tell which of these is the browser you are using.'}
		</Note>
	</Panel>

	<Machines
		devices={registry.data?.devices ?? []}
		{joined}
		{now}
		pending={registry.isPending || signIns.isPending || current.isPending}
		failed={registry.isError || signIns.isError}
	/>

	<Panel
		id="permissions"
		title="Marketplace permissions"
		description="Give Teachouse permission to use each marketplace on your machines. You only do this once."
	>
		{#if consents.isError}
			<p class="quiet">We could not load your permissions.</p>
		{/if}
		{#each permissions as row (row.marketplace)}
			<div class="acct-state-row">
				<span class="who">
					<span class="t">{row.name}</span>
					<span class="why"><StatusPill tone={row.pill.tone} label={row.pill.label} /></span>
				</span>
				{#if row.action === 'grant'}
					<Button tier="outline" small icon="shield-check" onclick={() => (grantingFor = row.marketplace)}>
						Grant
					</Button>
				{:else if row.action === 'withdraw'}
					<Button
						tier="outline"
						danger
						small
						disabled={withdrawing.isPending && withdrawing.variables === row.marketplace}
						onclick={() => withdraw(row.marketplace, row.name)}
					>
						{withdrawing.isPending && withdrawing.variables === row.marketplace
							? 'Withdrawing…'
							: 'Withdraw'}
					</Button>
				{/if}
			</div>
		{/each}
		{#if record.length > 0}
			<ul class="quiet acct-record">
				{#each record as line (line.key)}
					<li>{line.text}</li>
				{/each}
			</ul>
		{/if}
	</Panel>

	{#if grantingFor !== null}
		<ConsentDialog
			open={grantingFor !== null}
			marketplace={grantingFor}
			onAgreed={() => (grantingFor = null)}
			onClose={() => (grantingFor = null)}
		/>
	{/if}

	<Preferences />
</div>
