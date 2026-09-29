<script lang="ts">
	/** The card maintenance mode shows in place of a page: the console's, and
	 *  the signed-out screens' for a page that is not kept open. `onSignOut` is
	 *  offered only where there is a session to end. */
	let {
		message,
		onSignOut
	}: { message: string | null | undefined; onSignOut?: () => void } = $props();
</script>

<div class="auth-card">
	<h1>We’re doing some maintenance</h1>
	<p>Teachouse is taking a short break while we make some improvements. Your resources and listings are safe.</p>
	<p><strong>{message ?? 'We expect to be back shortly.'}</strong></p>
	<p>You can check the <a href="/status">status page</a> for the latest.</p>
	{#if onSignOut}
		<p class="claim-out"><button type="button" class="link" onclick={onSignOut}>Sign out</button></p>
	{/if}
</div>
