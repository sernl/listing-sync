<script lang="ts">
	import {
		SOCIAL_PROVIDERS,
		identity,
		resendVerification,
		signInWithProvider,
		signUpWithPassword,
		type SocialProvider
	} from '$lib/auth-client';
	import Turnstile from '$lib/Turnstile.svelte';
	import { TURNSTILE_SITE_KEY, captchaOptions, captchaPending } from '$lib/captcha';
	import { toast } from '$lib/toast';

	type Busy = 'register' | 'resend' | SocialProvider;

	/** Long enough for any display name a human types, short enough that the
	 * column is not a free-text sink: `auth.user.name` is unconstrained text. */
	const NAME_LIMIT = 120;

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

<div class="mx-auto max-w-md">
	{#if awaitingVerification !== null}
		<h1 class="mb-2 text-xl font-semibold">Check your email</h1>
		<p class="mb-4 text-sm text-slate-600">
			We sent a link to <span class="font-medium">{awaitingVerification}</span>.
			Confirm that address, then sign in.
		</p>
		<div class="flex gap-3">
			<button
				type="button"
				class="rounded bg-slate-900 px-4 py-2 text-sm text-white disabled:opacity-50"
				disabled={busy !== null}
				onclick={resend}
			>
				{busy === 'resend' ? 'Sending…' : 'Send it again'}
			</button>
			<a class="rounded border border-slate-300 px-4 py-2 text-sm" href="/login">Go to sign in</a>
		</div>
	{:else}
		<h1 class="mb-2 text-xl font-semibold">Create an account</h1>
		<p class="mb-4 text-sm text-slate-600">
			Signing up creates your organisation. You can invite people to it later.
		</p>

		<form onsubmit={register} class="flex flex-col gap-3">
			<label class="flex flex-col gap-1 text-sm" for="name">
				Name
				<input
					id="name"
					name="name"
					type="text"
					required
					maxlength={NAME_LIMIT}
					autocomplete="name"
					class="rounded border border-slate-300 px-3 py-2"
					bind:value={name}
				/>
			</label>
			<label class="flex flex-col gap-1 text-sm" for="email">
				Email
				<input
					id="email"
					name="email"
					type="email"
					required
					autocomplete="email"
					class="rounded border border-slate-300 px-3 py-2"
					bind:value={email}
				/>
			</label>
			<label class="flex flex-col gap-1 text-sm" for="password">
				Password
				<input
					id="password"
					name="password"
					type="password"
					required
					autocomplete="new-password"
					class="rounded border border-slate-300 px-3 py-2"
					bind:value={password}
				/>
			</label>
			<Turnstile bind:this={captcha} onToken={(token) => (captchaToken = token)} />
			<button
				class="rounded bg-slate-900 px-4 py-2 text-white disabled:opacity-50"
				disabled={busy !== null || challengePending}
			>
				{busy === 'register' ? 'Creating…' : 'Create account'}
			</button>
		</form>

		<div class="my-5 flex items-center gap-3 text-xs text-slate-400">
			<span class="h-px grow bg-slate-200"></span>
			or
			<span class="h-px grow bg-slate-200"></span>
		</div>

		<div class="flex flex-col gap-2">
			{#each SOCIAL_PROVIDERS as provider (provider.id)}
				<button
					type="button"
					class="rounded border border-slate-300 px-4 py-2 text-sm disabled:opacity-50"
					disabled={busy !== null}
					onclick={() => withProvider(provider.id)}
				>
					{busy === provider.id ? 'Redirecting…' : `Continue with ${provider.label}`}
				</button>
			{/each}
		</div>

		<p class="mt-6 text-sm text-slate-600">
			Already have an account? <a class="underline" href="/login">Sign in</a>.
		</p>
	{/if}
</div>
