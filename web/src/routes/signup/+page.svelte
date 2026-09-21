<script lang="ts">
	import {
		identity,
		resendVerification,
		signInWithProvider,
		signUpWithPassword
	} from '$lib/auth-client';
	import Button from '$lib/Button.svelte';
	import Field from '$lib/Field.svelte';
	import Turnstile from '$lib/Turnstile.svelte';
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
	 * `$lib/pages/account/intent`, which `/settings/subscription` spends. */
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

	// Registration never leads straight to the dashboard. The identity service
	// sends a verification link on sign-up, and the API refuses an assertion
	// whose address is unverified, so the honest next screen is the one that
	// says so rather than an exchange that would fail.
	async function register(event: SubmitEvent) {
		event.preventDefault();
		busy = 'register';
		try {
			const address = email.trim();
			const { error } = await signUpWithPassword(
				name.trim(),
				address,
				password,
				captchaOptions(captchaToken)
			);
			if (error) {
				captcha?.reset();
				toast('error', messageOf(error, 'That account could not be created.'));
				return;
			}
			password = '';
			awaitingVerification = (await identity())?.email ?? address;
		} finally {
			busy = null;
		}
	}

	async function withProvider(provider: SocialProvider) {
		busy = provider;
		const { error } = await signInWithProvider(provider);
		if (error) {
			busy = null;
			toast('error', messageOf(error, `Signing up with ${provider} is unavailable.`));
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
					? messageOf(error, 'The verification email could not be sent.')
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
		<p>Confirm the address we sent a link to, <b>{awaitingVerification}</b>.</p>
		<div class="actions">
			<Button
				tier="primary"
				disabled={busy !== null}
				reason={busy !== null ? 'A sign-up step is already running.' : undefined}
				onclick={resend}
			>
				{busy === 'resend' ? 'Sending…' : 'Send it again'}
			</Button>
			<Button tier="outline" href={signInHref}>Go to sign in</Button>
		</div>
	{:else}
		<h1>Create an account</h1>
		<p>Signing up creates your organisation.</p>

		<form onsubmit={register} class="form">
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
			<Button
				tier="primary"
				type="submit"
				disabled={busy !== null || challengePending}
				reason={busy !== null
					? 'A sign-up step is already running.'
					: challengePending
						? 'The challenge above has not been answered yet.'
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
						disabled={busy !== null}
						reason={busy !== null ? 'A sign-up step is already running.' : undefined}
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
