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

<div class="auth-card">
	{#if sent}
		<h1>Check your email</h1>
		<p>
			If <b>{email.trim()}</b> has an account, a link to choose a new password is on its way.
		</p>
		<div class="actions"><a class="btn" href="/login">Go to sign in</a></div>
	{:else}
		<h1>Reset your password</h1>
		<p>
			Give the address you signed up with and we will send a link for choosing a new password.
		</p>

		<form onsubmit={request} class="form">
			<label class="field" for="email">
				Email
				<input id="email" name="email" type="email" required autocomplete="username" bind:value={email} />
			</label>
			<Turnstile bind:this={captcha} onToken={(token) => (captchaToken = token)} />
			<button class="cta" disabled={busy || challengePending}>
				{busy ? 'Sending…' : 'Send the link'}
			</button>
		</form>

		<p class="auth-foot">
			Remembered it? <a class="link" href="/login">Sign in</a>.
		</p>
	{/if}
</div>
