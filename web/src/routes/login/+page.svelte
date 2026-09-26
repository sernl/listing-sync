<script lang="ts">
	import { onMount } from 'svelte';
	import { goto, invalidateAll } from '$app/navigation';
	import { ApiFailure } from '$lib/api';
	import {
		BridgeFailure,
		establishSession,
		identity,
		resendVerification,
		signInWithPasskey,
		signInWithPassword,
		signInWithProvider
	} from '$lib/auth-client';
	import Button from '$lib/Button.svelte';
	import Field from '$lib/Field.svelte';
	import Turnstile from '$lib/Turnstile.svelte';
	import { TURNSTILE_SITE_KEY, captchaOptions, captchaPending } from '$lib/captcha';
	import { ENABLED_SOCIAL_PROVIDERS, type SocialProvider } from '$lib/social-providers';
	import { toast } from '$lib/toast';
	import { page } from '$app/state';
	import { peekIntent, safeNext, safePrice, writeIntent } from '$lib/pages/account/intent';
	import '$lib/pages/account/signed-out.css';

	/** What the landing page asked for, if the sign-in link carried it. The
	 * price is written down before the browser leaves for the console,
	 * because `/settings/subscription` is the page that spends it and this
	 * one only passes it on. A social sign-in comes back here with the query
	 * gone, so the stored record is what answers then. */
	const intendedNext = $derived(safeNext(page.url.searchParams.get('next')));
	const intendedPrice = $derived(safePrice(page.url.searchParams.get('price')));

	$effect(() => {
		writeIntent({ next: intendedNext, price: intendedPrice });
	});

	/** Named here rather than in the markup: a build with no social provider
	 * offers no linked account, and saying otherwise sends the human looking
	 * for a button that is not there. */
	const SIGN_IN_PROMPT =
		ENABLED_SOCIAL_PROVIDERS.length > 0
			? 'Use your email, a passkey, or a linked account.'
			: 'Use your email or a passkey.';

	type Busy = 'password' | 'passkey' | 'resend' | 'resume' | SocialProvider;

	let email = $state('');
	let password = $state('');
	let busy = $state<Busy | null>(null);
	let awaitingVerification = $state<string | null>(null);
	let captchaToken = $state<string | null>(null);
	let captcha = $state<ReturnType<typeof Turnstile> | null>(null);
	const challengePending = $derived(captchaPending(TURNSTILE_SITE_KEY, captchaToken));

	function messageOf(error: unknown, fallback: string): string {
		if (error !== null && typeof error === 'object' && 'message' in error) {
			const message = (error as { message?: unknown }).message;
			if (typeof message === 'string' && message.length > 0) {
				return message;
			}
		}
		return fallback;
	}

	// The exchange, and the two places it can end other than signed in. An
	// unverified address is named because the browser knows it first-hand; the
	// API's own refusal stays blank on purpose and so gets a blank message.
	async function finish(fallbackAddress: string) {
		try {
			await establishSession();
			await invalidateAll();
			// The page the landing CTA asked for, from the link where it is
			// still in the query and from the stored record where a provider
			// redirect has taken it out. Peeked rather than read: the
			// checkout in the same record is not this page's to spend.
			await goto(intendedNext ?? peekIntent()?.next ?? '/');
		} catch (failure) {
			if (failure instanceof BridgeFailure && failure.refusal === 'unverified-email') {
				awaitingVerification = (await identity())?.email ?? fallbackAddress;
				return;
			}
			toast(
				'error',
				failure instanceof ApiFailure && failure.status === 401
					? 'Your sign-in was not accepted. Try again.'
					: 'We could not finish signing you in. Try again.'
			);
		}
	}

	// A social provider returns the browser here, and an expired API session
	// leaves the identity session standing. Both arrive holding a live
	// identity, so finishing the exchange on arrival is what makes them a
	// sign-in rather than a dead end.
	onMount(() => {
		void (async () => {
			const who = await identity();
			if (!who) {
				return;
			}
			if (!who.emailVerified) {
				awaitingVerification = who.email;
				return;
			}
			busy = 'resume';
			try {
				await finish(who.email);
			} finally {
				busy = null;
			}
		})();
	});

	async function withPassword(event: SubmitEvent) {
		event.preventDefault();
		busy = 'password';
		try {
			const { error } = await signInWithPassword(
				email.trim(),
				password,
				captchaOptions(captchaToken)
			);
			if (error) {
				captcha?.reset();
				toast('error', messageOf(error, 'That email and password did not match.'));
				return;
			}
			password = '';
			await finish(email.trim());
		} finally {
			busy = null;
		}
	}

	async function withPasskey() {
		busy = 'passkey';
		try {
			const { error } = await signInWithPasskey();
			if (error) {
				toast('error', messageOf(error, 'The passkey sign-in did not finish. Try again.'));
				return;
			}
			await finish('');
		} finally {
			busy = null;
		}
	}

	// On success the client's redirect plugin is already navigating to the
	// provider, so `busy` is deliberately left set.
	async function withProvider(provider: SocialProvider) {
		busy = provider;
		const { error } = await signInWithProvider(provider);
		if (error) {
			busy = null;
			toast('error', messageOf(error, `You cannot sign in with ${provider} right now.`));
		}
	}

	async function resend() {
		const address = awaitingVerification;
		if (address === null) {
			return;
		}
		busy = 'resend';
		try {
			const { error } = await resendVerification(address);
			toast(
				error ? 'error' : 'info',
				error
					? messageOf(error, 'We could not send the verification email. Try again.')
					: 'Verification email sent.'
			);
		} finally {
			busy = null;
		}
	}
</script>

<div class="auth-card acct-signed-out">
	{#if awaitingVerification !== null}
		<h1>Verify your email</h1>
		<p>Open the link we sent to <b>{awaitingVerification}</b>.</p>
		<div class="actions">
			<Button
				tier="primary"
				disabled={busy !== null}
				reason={busy !== null ? 'Wait for the current step to finish.' : undefined}
				onclick={resend}
			>
				{busy === 'resend' ? 'Sending…' : 'Send it again'}
			</Button>
			<Button
				tier="outline"
				disabled={busy !== null}
				reason={busy !== null ? 'Wait for the current step to finish.' : undefined}
				onclick={() => (awaitingVerification = null)}
			>
				Use a different account
			</Button>
		</div>
	{:else}
		<h1>Sign in</h1>
		<p>{busy === 'resume' ? 'Signing you in…' : SIGN_IN_PROMPT}</p>

		<form onsubmit={withPassword} class="form">
			<Field label="Email" id="email" required>
				<input id="email" name="email" type="email" required autocomplete="username" bind:value={email} />
			</Field>
			<Field label="Password" id="password" required>
				<input
					id="password"
					name="password"
					type="password"
					required
					autocomplete="current-password"
					bind:value={password}
				/>
			</Field>
			<Turnstile bind:this={captcha} onToken={(token) => (captchaToken = token)} />
			<Button
				tier="primary"
				type="submit"
				disabled={busy !== null || challengePending}
				reason={busy !== null
					? 'Wait for the current step to finish.'
					: challengePending
						? 'Complete the check above first.'
						: undefined}
			>
				{busy === 'password' ? 'Signing in…' : 'Sign in'}
			</Button>
		</form>

		<p class="auth-foot"><a class="link" href="/reset">Forgot your password?</a></p>

		<div class="divider">or</div>

		<div class="form">
			<Button
				tier="outline"
				disabled={busy !== null}
				reason={busy !== null ? 'Wait for the current step to finish.' : undefined}
				onclick={withPasskey}
			>
				{busy === 'passkey' ? 'Waiting for your passkey…' : 'Sign in with a passkey'}
			</Button>
			{#each ENABLED_SOCIAL_PROVIDERS as provider (provider.id)}
				<Button
					tier="outline"
					disabled={busy !== null}
					reason={busy !== null ? 'Wait for the current step to finish.' : undefined}
					onclick={() => withProvider(provider.id)}
				>
					{busy === provider.id ? 'Redirecting…' : `Continue with ${provider.label}`}
				</Button>
			{/each}
		</div>

		<p class="auth-foot">
			No account yet? <a class="link" href="/signup">Create one</a>.
		</p>
	{/if}
</div>
