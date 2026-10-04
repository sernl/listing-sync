<script lang="ts">
	import {
		identity,
		resendVerification,
		signUpWithPassword,
		signUpWithProvider
	} from '$lib/auth-client';
	import Banner from '$lib/Banner.svelte';
	import Button from '$lib/Button.svelte';
	import ConsentBoxes from '$lib/ConsentBoxes.svelte';
	import Field from '$lib/Field.svelte';
	import Turnstile from '$lib/Turnstile.svelte';
	import { consentBody, socialConsentError } from '$lib/legal';
	import { TURNSTILE_SITE_KEY, captchaOptions, captchaPending } from '$lib/captcha';
	import { ENABLED_SOCIAL_PROVIDERS, type SocialProvider } from '$lib/social-providers';
	import { toast } from '$lib/toast';
	import { page } from '$app/state';
	import { safeNext, safePrice, writeIntent } from '$lib/pages/account/intent';
	import '$lib/pages/account/signed-out.css';

	type Busy = 'register' | 'resend' | SocialProvider;

	/** Long enough for any display name a human types, short enough that the
	 * column is not a free-text sink: `auth.user.name` is unconstrained text. */
	const NAME_LIMIT = 120;

	/** What the landing page asked for, carried across the account it has to
	 * create first. `next` is where to land; `price` is the checkout to open
	 * on arrival. Both are written down as well as carried in the sign-in
	 * link, because the verification link is often opened in another tab and
	 * a social sign-up leaves through a provider redirect — see
	 * `$lib/pages/account/intent`, which `/settings/billing` spends. */
	const intendedNext = $derived(safeNext(page.url.searchParams.get('next')));
	const intendedPrice = $derived(safePrice(page.url.searchParams.get('price')));

	const signInHref = $derived.by(() => {
		const query = new URLSearchParams();
		if (intendedNext !== null) query.set('next', intendedNext);
		if (intendedPrice !== null) query.set('price', intendedPrice);
		const suffix = query.toString();
		return suffix === '' ? '/login' : `/login?${suffix}`;
	});

	$effect(() => {
		writeIntent({ next: intendedNext, price: intendedPrice });
	});

	let name = $state('');
	let email = $state('');
	let password = $state('');
	let terms = $state(false);
	let age = $state(false);
	let busy = $state<Busy | null>(null);
	let awaitingVerification = $state<string | null>(null);
	let captchaToken = $state<string | null>(null);
	let captcha = $state<ReturnType<typeof Turnstile> | null>(null);
	const challengePending = $derived(captchaPending(TURNSTILE_SITE_KEY, captchaToken));
	/** What is sent with the account, or null until both boxes are ticked:
	 *  nothing on this page makes an account without it. */
	const agreement = $derived(consentBody({ terms, age }));
	/** Why a provider sign-up came back here rather than signed in. */
	const returnedRefusal = $derived(socialConsentError(page.url.searchParams.get('error')));

	function messageOf(error: unknown, fallback: string): string {
		if (error !== null && typeof error === 'object' && 'message' in error) {
			const message = (error as { message?: unknown }).message;
			if (typeof message === 'string' && message.length > 0) {
				return message;
			}
		}
		return fallback;
	}

	// Registration never leads straight to the dashboard. The identity service
	// sends a verification link on sign-up, and the API refuses an assertion
	// whose address is unverified, so the honest next screen is the one that
	// says so rather than an exchange that would fail.
	async function register(event: SubmitEvent) {
		event.preventDefault();
		if (agreement === null) {
			return;
		}
		busy = 'register';
		try {
			const address = email.trim();
			const { error } = await signUpWithPassword(
				name.trim(),
				address,
				password,
				agreement,
				captchaOptions(captchaToken)
			);
			if (error) {
				captcha?.reset();
				toast('error', messageOf(error, 'We could not create that account. Try again.'));
				return;
			}
			password = '';
			awaitingVerification = (await identity())?.email ?? address;
		} finally {
			busy = null;
		}
	}

	async function withProvider(provider: SocialProvider) {
		if (agreement === null) {
			return;
		}
		busy = provider;
		const { error } = await signUpWithProvider(provider, agreement);
		if (error) {
			busy = null;
			toast('error', messageOf(error, `You cannot sign up with ${provider} right now.`));
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
				error ? 'error' : 'success',
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
		<h1>Check your email</h1>
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
			<Button tier="outline" href={signInHref}>Go to sign in</Button>
		</div>
	{:else}
		<h1>Create an account</h1>
		<p>Set up your Teachouse account.</p>

		{#if returnedRefusal !== null}
			<div id="signup-refusal" class="auth-refusal">
				<Banner tone="bad">{returnedRefusal}</Banner>
			</div>
		{/if}

		<form id="signup-form" onsubmit={register} class="form">
			<Field label="Name" id="name" required>
				<input
					id="name"
					name="name"
					type="text"
					required
					maxlength={NAME_LIMIT}
					autocomplete="name"
					bind:value={name}
				/>
			</Field>
			<Field label="Email" id="email" required>
				<input id="email" name="email" type="email" required autocomplete="email" bind:value={email} />
			</Field>
			<Field label="Password" id="password" required>
				<input
					id="password"
					name="password"
					type="password"
					required
					autocomplete="new-password"
					bind:value={password}
				/>
			</Field>
			<Turnstile bind:this={captcha} onToken={(token) => (captchaToken = token)} />
			<ConsentBoxes bind:terms bind:age disabled={busy !== null} idPrefix="signup-consent" />
			<Button
				tier="primary"
				type="submit"
				disabled={busy !== null || challengePending || agreement === null}
				reason={busy !== null
					? 'Wait for the current step to finish.'
					: challengePending
						? 'Complete the check above first.'
						: agreement === null
							? 'Tick both boxes above first.'
							: undefined}
			>
				{busy === 'register' ? 'Creating…' : 'Create account'}
			</Button>
		</form>

		{#if ENABLED_SOCIAL_PROVIDERS.length > 0}
			<div class="divider">or</div>
			<div class="form">
				{#each ENABLED_SOCIAL_PROVIDERS as provider (provider.id)}
					<Button
						tier="outline"
						disabled={busy !== null || agreement === null}
						reason={busy !== null
							? 'Wait for the current step to finish.'
							: agreement === null
								? 'Tick both boxes above first.'
								: undefined}
						onclick={() => withProvider(provider.id)}
					>
						{busy === provider.id ? 'Redirecting…' : `Continue with ${provider.label}`}
					</Button>
				{/each}
			</div>
		{/if}

		<p class="auth-foot">
			Already have an account? <a class="link" href="/login">Sign in</a>.
		</p>
	{/if}
</div>
