<script lang="ts">
	import Turnstile from '$lib/Turnstile.svelte';
	import { requestPasswordReset } from '$lib/auth-client';
	import { TURNSTILE_SITE_KEY, captchaOptions, captchaPending } from '$lib/captcha';
	import { toast } from '$lib/toast';

	let email = $state('');
	let busy = $state(false);
	let sent = $state(false);
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

	// The outcome says nothing about whether the address is registered, because
	// the identity service says nothing either. A refusal is still reported: a
	// spent challenge or an unreachable service is a fact about this browser,
	// not about the address, and a page that swallowed it would leave the human
	// waiting for an email nobody sent.
	async function request(event: SubmitEvent) {
		event.preventDefault();
		busy = true;
		try {
			const { error } = await requestPasswordReset(email.trim(), captchaOptions(captchaToken));
			if (error) {
				captcha?.reset();
				toast('error', messageOf(error, 'The reset link could not be requested.'));
				return;
			}
			sent = true;
		} finally {
			busy = false;
		}
	}
</script>

<div class="mx-auto max-w-md">
	{#if sent}
		<h1 class="mb-2 text-xl font-semibold">Check your email</h1>
		<p class="mb-4 text-sm text-slate-600">
			If <span class="font-medium">{email.trim()}</span> has an account, a link to choose a
			new password is on its way.
		</p>
		<a class="rounded border border-slate-300 px-4 py-2 text-sm" href="/login">Go to sign in</a>
	{:else}
		<h1 class="mb-2 text-xl font-semibold">Reset your password</h1>
		<p class="mb-4 text-sm text-slate-600">
			Give the address you signed up with and we will send a link for choosing a new password.
		</p>

		<form onsubmit={request} class="flex flex-col gap-3">
			<label class="flex flex-col gap-1 text-sm" for="email">
				Email
				<input
					id="email"
					name="email"
					type="email"
					required
					autocomplete="username"
					class="rounded border border-slate-300 px-3 py-2"
					bind:value={email}
				/>
			</label>
			<Turnstile bind:this={captcha} onToken={(token) => (captchaToken = token)} />
			<button
				class="rounded bg-slate-900 px-4 py-2 text-white disabled:opacity-50"
				disabled={busy || challengePending}
			>
				{busy ? 'Sending…' : 'Send the link'}
			</button>
		</form>

		<p class="mt-6 text-sm text-slate-600">
			Remembered it? <a class="underline" href="/login">Sign in</a>.
		</p>
	{/if}
</div>
