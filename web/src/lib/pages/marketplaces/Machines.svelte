<script lang="ts">
	import { createMutation, useQueryClient } from '@tanstack/svelte-query';
	import { api, type DeviceView } from '$lib/api';
	import { revokeBrowserSession } from '$lib/browser-sessions';
	import Button from '$lib/Button.svelte';
	import { type BrowserSession, matchNote, type Merged } from '$lib/device-merge';
	import { deviceFootnote, deviceRows, deviceSummary } from '$lib/devices-view';
	import { agoLabel } from '$lib/elapsed';
	import { MARKETPLACE_NAME } from '$lib/platforms';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import { toast } from '$lib/toast';
	import { osLabel, sessionLabel, sessionTone, sessionWords } from './machines';

	let {
		devices,
		joined,
		now,
		pending,
		failed
	}: {
		devices: readonly DeviceView[];
		joined: Merged;
		now: number;
		pending: boolean;
		failed: boolean;
	} = $props();

	const queryClient = useQueryClient();
	const summary = $derived(deviceSummary(deviceRows(devices, now)));

	/** Signing a machine out is two acts, and the page performs both: our
	 *  registry marks the device revoked, and the identity service ends the
	 *  browser sign-in we matched to it. Neither implies the other — the two
	 *  planes are separate by charter — so one failing must not skip the other,
	 *  and the result says which parts actually happened. */
	const signingOut = createMutation(() => ({
		mutationFn: async (input: { device: DeviceView; session: BrowserSession | null }) => {
			const endingDevice = api.revokeDevice(input.device.id).then(
				() => true,
				() => false
			);
			const endingSignIn =
				input.session === null
					? Promise.resolve(null)
					: revokeBrowserSession(input.session.token).then(
							() => true,
							() => false
						);
			const [deviceEnded, signInEnded] = await Promise.all([endingDevice, endingSignIn]);
			return { deviceEnded, signInEnded };
		},
		onSuccess: async (done: { deviceEnded: boolean; signInEnded: boolean | null }) => {
			if (!done.deviceEnded) {
				toast('error', 'The machine was not signed out, so it still holds its marketplace logins.');
			} else if (done.signInEnded === false) {
				toast(
					'error',
					'The machine was signed out, but its browser sign-in could not be ended. Try that one again below.'
				);
			} else {
				toast(
					'info',
					'Signed out. This machine forgets its marketplace logins when it next checks in.'
				);
			}
			await Promise.all([
				queryClient.invalidateQueries({ queryKey: queryKeys.devices }),
				queryClient.invalidateQueries({ queryKey: queryKeys.browserSessions })
			]);
		},
		onError: () => {
			toast('error', 'The machine was not signed out.');
		}
	}));

	function signOut(device: DeviceView, session: BrowserSession | null) {
		const sure = confirm(
			`Sign "${device.name}" out?\n\n` +
				'It stops syncing and forgets its marketplace logins the next time it reaches ' +
				'us. Until then it still holds them, because they are on that machine and ' +
				'never on our servers. Signing in again there restores it.'
		);
		if (sure) {
			signingOut.mutate({ device, session });
		}
	}

	function busy(device: DeviceView): boolean {
		return signingOut.isPending && signingOut.variables?.device.id === device.id;
	}
</script>

<section id="machines">
	<div class="mp-sect">
		<h2>Your machines</h2>
		<p>
			Each machine running the desktop app appears here. Your marketplace logins stay on that
			machine and never reach our servers, so this list only says what each one reports
			holding.
		</p>
	</div>

	{#if pending}
		<p class="quiet">Loading…</p>
	{:else if failed}
		<p class="quiet">Your machines could not be listed.</p>
	{:else if joined.rows.length === 0}
		<div class="mp-card">
			<p class="mp-body">
				No machine is registered yet. Install the desktop app and sign in on it, and it
				appears here.
			</p>
			<div class="mp-foot"><a class="go" href="#downloads">Downloads <span aria-hidden="true">→</span></a></div>
		</div>
	{:else}
		<div class="mp-card">
			{#each joined.rows as row (row.device.id)}
				<div class="mp-machine">
					<div class="who">
						<span class="t">{row.device.name}</span>
						{#if row.isCurrent}<StatusPill tone="ok" label="This browser" />{/if}
						{#if row.device.revoked_at !== null}
							<StatusPill tone="bad" label="Signed out" />
						{/if}
						{#if row.device.revoked_at === null}
							<span class="act">
								<Button
									tier="outline"
									danger
									small
									disabled={busy(row.device)}
									reason={busy(row.device) ? 'The sign-out is running.' : undefined}
									onclick={() => signOut(row.device, row.session)}
								>
									{busy(row.device) ? 'Signing out…' : 'Sign out'}
								</Button>
							</span>
						{/if}
					</div>
					<p class="spec">
						{osLabel(row.device)} · {row.device.arch} · app {row.device.app_version} · last
						seen {agoLabel(row.device.last_seen_at, now)}
					</p>
					<p class="spec">{matchNote(row.confidence)}</p>

					{#if row.device.wipe_outstanding}
						<p class="mp-warned">
							Signed out {agoLabel(row.device.revoked_at ?? now, now)}, and this machine has
							not checked in since, so it still holds the logins below. If it never checks
							in again, they stay there until each marketplace expires them — we cannot
							remove them, because we have never held them.
						</p>
					{/if}

					{#if row.device.sessions.length === 0}
						<p class="spec">No marketplace login on this machine.</p>
					{:else}
						<div class="mp-held">
							{#each row.device.sessions as session (session.marketplace)}
								<div class="one">
									<span>{MARKETPLACE_NAME[session.marketplace]}</span>
									<StatusPill
									tone={sessionTone(session.status)}
									label={sessionLabel(session.status)}
								/>
									<span>{sessionWords(session)}</span>
									<span class="when">linked {agoLabel(session.linked_at, now)}</span>
								</div>
							{/each}
						</div>
					{/if}
				</div>
			{/each}
			<p class="mp-body">{deviceFootnote(summary)}</p>
		</div>
	{/if}
</section>
