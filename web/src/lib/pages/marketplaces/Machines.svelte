<script lang="ts">
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { api, type DeviceView } from '$lib/api';
	import { revokeBrowserSession } from '$lib/browser-sessions';
	import Button from '$lib/Button.svelte';
	import { type CheckInHere, checkInHere, desktopInvoker } from '$lib/desktop';
	import { type BrowserSession, matchNote, type Merged } from '$lib/device-merge';
	import { deviceFootnote, deviceRows, deviceSummary } from '$lib/devices-view';
	import { entitlementRead, limitOf } from '$lib/entitlement-read';
	import { agoLabel } from '$lib/elapsed';
	import MarketplaceMark from '$lib/MarketplaceMark.svelte';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import { toast } from '$lib/toast';
	import {
		signBackInRefusal,
		SIGN_BACK_IN
	} from '$lib/machine-here';
	import { machineHere } from '$lib/machine.svelte';
	import {
		checkInControl,
		checkInNote,
		inListOrder,
		loginWords,
		machineWords,
		osLabel,
		sessionLabel,
		sessionTone,
		sessionWords,
		sourcedFileWords
	} from './machines';

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

	/** The current installations first, the signed-out records in history.
	 *
	 *  Identified by installation id: `machineHere.where.device` is what this
	 *  console's own check-in reported, and a display-name match would put one
	 *  machine's actions on another's row. */
	const split = $derived(inListOrder(joined.rows, machineHere.where.device?.id ?? null));

	// Read once: whether this console is running inside the application does not
	// change while the page is open.
	const invoke = desktopInvoker();

	/** The last check-in this panel asked for, or null before any was asked.
	 *
	 *  Only this panel's own: the console registers the machine once per load
	 *  from `Console.svelte`, and that answer is deliberately not shown, because
	 *  a seller who did not ask is owed nothing. A seller who pressed the button
	 *  did ask. */
	let asked = $state<CheckInHere | null>(null);

	/** A machine has no other way to make the seller's list current right now.
	 *
	 *  The application checks in every five minutes and discovers work within
	 *  seconds of it appearing, so this is a refresh rather than the only way
	 *  anything moves — which is what it was when the only cadence was hourly.
	 *  On a phone there is still no background timer at all: Android's Doze
	 *  stops one, so between resumes the list cannot become current by
	 *  waiting.
	 *
	 *  Idempotent on the server's own upsert, so pressing it twice is a
	 *  refresh rather than a second machine. */
	const checkingIn = createMutation(() => ({
		mutationFn: () => checkInHere(invoke),
		onSuccess: async (answer: CheckInHere) => {
			asked = answer;
			// The same pair the disconnect invalidates: a check-in replaces this
			// machine's reported session list on the server, and `derive_link`
			// re-decides the connection from it in the same transaction, so the
			// tiles above and the rows below are both answered by this call.
			await Promise.all([
				queryClient.invalidateQueries({ queryKey: queryKeys.devices }),
				queryClient.invalidateQueries({ queryKey: queryKeys.connections })
			]);
		}
	}));

	// The check-in is what registers this machine, so the device cap is what
	// refuses it. The plan's refusal takes precedence over the in-flight one:
	// at the cap the control never runs, so "the check-in is running" would be
	// a sentence about something that is not happening.
	const plan = createQuery(() => entitlementRead);
	const capped = $derived(limitOf(plan.data, 'devices'));
	const control = $derived(checkInControl(checkingIn.isPending));
	const note = $derived(asked === null ? null : checkInNote(asked));

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
				'never on our servers. To bring it back, sign it back in from that machine: ' +
				'signing in to Teachouse there is not enough on its own.'
		);
		if (sure) {
			signingOut.mutate({ device, session });
		}
	}

	function busy(device: DeviceView): boolean {
		return signingOut.isPending && signingOut.variables?.device.id === device.id;
	}

	/** Sign this machine back in, from its own row.
	 *
	 *  The act the founder had no way to perform. Signing a machine out marks
	 *  it revoked and nothing ever clears that mark: the machine keeps its
	 *  identity, keeps checking in, and wipes its marketplace logins on every
	 *  cycle, so every Connect on it fails after the password has been typed.
	 *  Signing in to Teachouse again on that machine did not help, because the
	 *  app only re-registers when the server has forgotten it entirely.
	 *
	 *  Offered on this machine's row alone: the restore is the seller's
	 *  explicit act at the machine, which is what keeps a sign-out a decision
	 *  rather than a delay. */
	const signingBackIn = createMutation(() => ({
		mutationFn: () => machineHere.signBackIn(),
		onSuccess: async () => {
			toast('info', 'This machine is signed back in. Connect your marketplaces again on it.');
			await Promise.all([
				queryClient.invalidateQueries({ queryKey: queryKeys.devices }),
				queryClient.invalidateQueries({ queryKey: queryKeys.connections })
			]);
		},
		onError: (failure: unknown) => {
			toast('error', signBackInRefusal(failure));
		}
	}));
</script>

<section id="machines">
	<div class="mp-sect">
		<div class="head">
			<h2>Your machines</h2>
			{#if invoke !== null}
				<Button
					tier="outline"
					small
					disabled={capped !== null || control.disabled}
					reason={capped ?? control.reason}
					onclick={() => checkingIn.mutate()}
				>
					{control.label}
				</Button>
			{/if}
		</div>
		<p>
			Each computer and phone running the Teachouse app appears here. Your marketplace logins
			stay on that machine and never reach our servers, so this list only says what each one
			reports holding.
		</p>
		{#if note !== null}
			<p class="mp-warned">{note}</p>
		{/if}
	</div>

	{#if pending}
		<p class="quiet">Loading…</p>
	{:else if failed}
		<p class="quiet">We could not list your machines.</p>
	{:else if joined.rows.length === 0}
		<div class="mp-card">
			<p class="mp-body">
				No machine is registered yet. Install the Teachouse app on a computer or a phone and
				sign in on it, and it appears here.
			</p>
			<div class="mp-foot"><a class="go" href="#downloads">Downloads <span aria-hidden="true">→</span></a></div>
		</div>
	{:else}
		<div class="mp-card">
			{#each split.current as row (row.device.id)}
				{@const here = row.device.id === machineHere.where.device?.id}
				<div class="mp-machine">
					<div class="who">
						<span class="t">{row.device.name}</span>
						<!-- Which row is the machine the seller is reading this on.
						     Distinct from "This browser", which the merge decides from
						     the browser session: a seller in the app is in both, and a
						     seller in a browser on a machine that also runs the app is
						     in the second alone. Without it the founder's own list gave
						     him no way to tell which of two signed-out machines was the
						     one in his hands. -->
						{#if here}<StatusPill tone="ok" label="This machine" />{/if}
						{#if row.isCurrent}<StatusPill tone="ok" label="This browser" />{/if}
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
					</div>
					<!-- Three separate facts, each on its own line and none standing
					     in for another: when we last heard from this machine, what
					     logins are saved on it, and which browser session we matched
					     it to. The defect being repaired is one line that read as all
					     of them at once. A fourth line appears only where this
					     installation's version actually withholds something, and it
					     names that one thing rather than calling the machine old. -->
					<p class="spec">{machineWords(row.device, now)}</p>
					<p class="spec">{loginWords(row.device)}</p>
					{#if sourcedFileWords(row.device) !== null}
						<p class="mp-warned">{sourcedFileWords(row.device)}</p>
					{/if}
					<p class="spec">{matchNote(row.confidence)}</p>

					{#if row.device.sessions.length > 0}
						<div class="mp-held">
							{#each row.device.sessions as session (session.marketplace)}
								<div class="one">
									<MarketplaceMark marketplace={session.marketplace} size={18} />
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

					<!-- The technical identity, in a disclosure. A seller reads the
					     name and the sentences above; support and the seller
					     comparing two reinstalls need the installation id, and it is
					     the only thing that tells two machines of one name apart. -->
					<details class="spec">
						<summary>Details</summary>
						<p class="spec">
							{osLabel(row.device)} · {row.device.arch} · app {row.device.app_version}
						</p>
						<p class="spec">Installation {row.device.id}</p>
						<p class="spec">First seen {agoLabel(row.device.first_seen_at, now)}</p>
					</details>
				</div>
			{/each}
			<p class="mp-body">{deviceFootnote(summary)}</p>
		</div>

		{#if split.history.length > 0}
			<!-- Signed-out records, kept rather than deleted or merged. A
			     reinstall mints a new identity, so a seller who has reinstalled
			     keeps a record per installation; grouping them here is
			     presentation and never a claim that they are one machine. The
			     sign-back-in act is offered on the record for the machine the
			     seller is standing at, because only somebody at it can perform
			     it. -->
			<details class="mp-card">
				<summary>Earlier installations and sign-outs ({split.history.length})</summary>
				{#each split.history as row (row.device.id)}
					{@const here = row.device.id === machineHere.where.device?.id}
					<div class="mp-machine">
						<div class="who">
							<span class="t">{row.device.name}</span>
							{#if here}<StatusPill tone="ok" label="This machine" />{/if}
							<StatusPill tone="bad" label="Signed out" />
							{#if here}
								<span class="act">
									<Button
										tier="outline"
										small
										disabled={machineHere.restoring}
										reason={machineHere.restoring ? 'The sign-in is running.' : undefined}
										onclick={() => signingBackIn.mutate()}
									>
										{machineHere.restoring ? 'Signing in…' : SIGN_BACK_IN}
									</Button>
								</span>
							{/if}
						</div>
						<p class="spec">{machineWords(row.device, now)}</p>

						{#if row.device.wipe_outstanding}
							<p class="mp-warned">
								Signed out {agoLabel(row.device.revoked_at ?? now, now)}, and this machine has
								not checked in since, so it still holds the logins below. If it never checks
								in again, they stay there until each marketplace expires them — we cannot
								remove them, because we have never held them.
							</p>
						{/if}

						{#if row.device.sessions.length > 0}
							<div class="mp-held">
								{#each row.device.sessions as session (session.marketplace)}
									<div class="one">
										<MarketplaceMark marketplace={session.marketplace} size={18} />
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

						<details class="spec">
							<summary>Details</summary>
							<p class="spec">
								{osLabel(row.device)} · {row.device.arch} · app {row.device.app_version}
							</p>
							<p class="spec">Installation {row.device.id}</p>
							<p class="spec">First seen {agoLabel(row.device.first_seen_at, now)}</p>
						</details>
					</div>
				{/each}
			</details>
		{/if}
	{/if}
</section>

<style>
	/* The heading and its one control on a line, wrapping at 390 rather than
	   squeezing the button. Here rather than in `marketplaces.css` because the
	   panel is the only section on the page that carries a control beside its
	   heading. */
	.head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
		flex-wrap: wrap;
	}

	/* The two disclosures: the per-machine Details and the history list. A
	   summary that did not read as a control was pressed by nobody in the
	   founder's own walkthrough. */
	summary {
		cursor: pointer;
		font-weight: 600;
	}
</style>
